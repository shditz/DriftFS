use crate::error::{MetadataError, Result};
use crate::model::{StagingJournalEntry, StagingState, StoredObject, SyncStatus};
use crate::schema::run_migrations;
use driftfs_core::{AccountId, FileId};
use driftfs_provider::{Change, ObjectKind, ObjectMetadata};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use std::path::Path;
use std::sync::Mutex;

pub enum ConnectionGuard<'a> {
    Reader(std::sync::MutexGuard<'a, Connection>),
    Writer(std::sync::MutexGuard<'a, Connection>),
}

impl<'a> std::ops::Deref for ConnectionGuard<'a> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Reader(guard) => guard,
            Self::Writer(guard) => guard,
        }
    }
}

pub struct MetadataStore {
    writer: Mutex<Connection>,
    reader: Option<Mutex<Connection>>,
}

impl MetadataStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MetadataError::InvalidState(e.to_string()))?;
        }

        let mut writer = Connection::open(path)?;
        Self::apply_pragmas(&mut writer)?;
        run_migrations(&mut writer)?;

        let mut reader = Connection::open(path)?;
        Self::apply_pragmas(&mut reader)?;

        let store = Self {
            writer: Mutex::new(writer),
            reader: Some(Mutex::new(reader)),
        };
        let _ = store.normalize_root_orphans();
        Ok(store)
    }

    pub fn normalize_root_orphans(&self) -> Result<usize> {
        let conn = self.lock_writer()?;
        let count = conn.execute(
            "UPDATE objects SET parent_id = NULL 
             WHERE parent_id IS NOT NULL 
               AND parent_id NOT IN (SELECT id FROM objects)",
            [],
        )?;
        Ok(count)
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        run_migrations(&mut conn)?;

        Ok(Self {
            writer: Mutex::new(conn),
            reader: None,
        })
    }

    fn apply_pragmas(conn: &mut Connection) -> Result<()> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(())
    }

    fn lock_writer(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.writer.lock().map_err(|_| {
            MetadataError::InvalidState("metadata writer connection lock poisoned".into())
        })
    }

    fn lock_reader(&self) -> Result<ConnectionGuard<'_>> {
        if let Some(ref reader) = self.reader {
            let guard = reader.lock().map_err(|_| {
                MetadataError::InvalidState("metadata reader connection lock poisoned".into())
            })?;
            Ok(ConnectionGuard::Reader(guard))
        } else {
            let guard = self.lock_writer()?;
            Ok(ConnectionGuard::Writer(guard))
        }
    }

    pub fn upsert_object(&self, meta: &ObjectMetadata) -> Result<()> {
        let mut conn = self.lock_writer()?;
        let tx = conn.transaction()?;
        Self::tx_upsert_object(&tx, meta)?;
        tx.commit()?;
        Ok(())
    }

    pub fn batch_upsert_objects(&self, objects: &[ObjectMetadata]) -> Result<()> {
        let mut conn = self.lock_writer()?;
        let tx = conn.transaction()?;
        for obj in objects {
            Self::tx_upsert_object(&tx, obj)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_object(&self, id: &FileId) -> Result<Option<StoredObject>> {
        let conn = self.lock_reader()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, parent_id, name, remote_name, kind, size_bytes,
                    mime_type, created_at, modified_at, version, sync_status, deleted
             FROM objects
             WHERE id = ?1 AND deleted = 0",
        )?;

        stmt.query_row(params![id.0], row_to_stored_object)
            .optional()
            .map_err(MetadataError::from)
    }

    pub fn lookup_by_name(
        &self,
        parent_id: Option<&FileId>,
        name: &str,
    ) -> Result<Option<StoredObject>> {
        let conn = self.lock_reader()?;
        let parent_str = parent_id.map(|p| p.0.as_str());

        let mut stmt = conn.prepare_cached(
            "SELECT id, parent_id, name, remote_name, kind, size_bytes,
                    mime_type, created_at, modified_at, version, sync_status, deleted
             FROM objects
             WHERE parent_id IS ?1 AND name = ?2 COLLATE NOCASE AND deleted = 0",
        )?;

        stmt.query_row(params![parent_str, name], row_to_stored_object)
            .optional()
            .map_err(MetadataError::from)
    }

    pub fn list_children(&self, parent_id: Option<&FileId>) -> Result<Vec<StoredObject>> {
        let conn = self.lock_reader()?;
        let parent_str = parent_id.map(|p| p.0.as_str());

        let mut stmt = conn.prepare_cached(
            "SELECT id, parent_id, name, remote_name, kind, size_bytes,
                    mime_type, created_at, modified_at, version, sync_status, deleted
             FROM objects
             WHERE parent_id IS ?1 AND deleted = 0
             ORDER BY kind DESC, name COLLATE NOCASE ASC",
        )?;

        let rows = stmt.query_map(params![parent_str], row_to_stored_object)?;
        let mut objects = Vec::new();
        for row in rows {
            objects.push(row?);
        }

        Ok(objects)
    }

    pub fn mark_deleted(&self, id: &FileId) -> Result<()> {
        let conn = self.lock_writer()?;
        conn.execute(
            "UPDATE objects SET deleted = 1 WHERE id = ?1",
            params![id.0],
        )?;
        Ok(())
    }

    pub fn batch_apply_changes(
        &self,
        changes: &[Change],
        new_checkpoint: Option<&str>,
        account_id: &AccountId,
    ) -> Result<()> {
        let mut conn = self.lock_writer()?;
        let tx = conn.transaction()?;

        for change in changes {
            match change {
                Change::Upsert(meta) => {
                    Self::tx_upsert_object(&tx, meta)?;
                }
                Change::Delete { id } => {
                    tx.execute(
                        "UPDATE objects SET deleted = 1 WHERE id = ?1",
                        params![id.0],
                    )?;
                }
            }
        }

        if let Some(token) = new_checkpoint {
            tx.execute(
                "INSERT INTO sync_checkpoints (account_id, checkpoint_token, last_sync_at)
                 VALUES (?1, ?2, datetime('now'))
                 ON CONFLICT(account_id) DO UPDATE SET
                     checkpoint_token = excluded.checkpoint_token,
                     last_sync_at = excluded.last_sync_at",
                params![account_id.0, token],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn get_checkpoint(&self, account_id: &AccountId) -> Result<Option<String>> {
        let conn = self.lock_reader()?;
        let mut stmt = conn.prepare_cached(
            "SELECT checkpoint_token FROM sync_checkpoints WHERE account_id = ?1",
        )?;

        stmt.query_row(params![account_id.0], |row| row.get(0))
            .optional()
            .map_err(MetadataError::from)
    }

    pub fn set_checkpoint(&self, account_id: &AccountId, token: &str) -> Result<()> {
        let conn = self.lock_writer()?;
        conn.execute(
            "INSERT INTO sync_checkpoints (account_id, checkpoint_token, last_sync_at)
             VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(account_id) DO UPDATE SET
                 checkpoint_token = excluded.checkpoint_token,
                 last_sync_at = excluded.last_sync_at",
            params![account_id.0, token],
        )?;
        Ok(())
    }

    pub fn purge_tombstones(&self) -> Result<usize> {
        let conn = self.lock_writer()?;
        let count = conn.execute("DELETE FROM objects WHERE deleted = 1", [])?;
        Ok(count)
    }

    fn tx_upsert_object(tx: &Transaction, meta: &ObjectMetadata) -> Result<()> {
        let parent_str = meta.parent_id.as_ref().map(|p| p.0.as_str());
        let sanitized_name = meta.name.replace(['/', '\\'], "_");
        let local_name =
            Self::resolve_unique_name(tx, meta.parent_id.as_ref(), &sanitized_name, &meta.id)?;
        let kind_str = match meta.kind {
            ObjectKind::Directory => "directory",
            ObjectKind::File => "file",
        };
        let size_val = meta.size_bytes.map(|s| s as i64);

        tx.execute(
            "INSERT INTO objects (
                id, parent_id, name, remote_name, kind, size_bytes,
                mime_type, created_at, modified_at, version, sync_status, deleted
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'synced', 0)
            ON CONFLICT(id) DO UPDATE SET
                parent_id = excluded.parent_id,
                name = excluded.name,
                remote_name = excluded.remote_name,
                kind = excluded.kind,
                size_bytes = excluded.size_bytes,
                mime_type = excluded.mime_type,
                created_at = excluded.created_at,
                modified_at = excluded.modified_at,
                version = excluded.version,
                sync_status = excluded.sync_status,
                deleted = 0",
            params![
                meta.id.0,
                parent_str,
                local_name,
                meta.name,
                kind_str,
                size_val,
                meta.mime_type,
                meta.created_at,
                meta.modified_at,
                meta.version,
            ],
        )?;

        Ok(())
    }

    fn resolve_unique_name(
        tx: &Transaction,
        parent_id: Option<&FileId>,
        remote_name: &str,
        file_id: &FileId,
    ) -> Result<String> {
        let parent_str = parent_id.map(|p| p.0.as_str());

        let existing_owner: Option<String> = tx
            .query_row(
                "SELECT id FROM objects WHERE parent_id IS ?1 AND name = ?2 COLLATE NOCASE AND deleted = 0",
                params![parent_str, remote_name],
                |r| r.get(0),
            )
            .optional()?;

        match existing_owner {
            None => Ok(remote_name.to_string()),
            Some(existing_id) if existing_id == file_id.0 => Ok(remote_name.to_string()),
            Some(_) => {
                let (base, ext) = split_name_ext(remote_name);
                let mut counter = 1;

                loop {
                    let candidate = format!("{base} ({counter}){ext}");
                    let collision: Option<String> = tx
                        .query_row(
                            "SELECT id FROM objects WHERE parent_id IS ?1 AND name = ?2 COLLATE NOCASE AND deleted = 0",
                            params![parent_str, candidate],
                            |r| r.get(0),
                        )
                        .optional()?;

                    match collision {
                        None => return Ok(candidate),
                        Some(col_id) if col_id == file_id.0 => return Ok(candidate),
                        Some(_) => counter += 1,
                    }
                }
            }
        }
    }

    pub fn rename_object(&self, id: &FileId, new_name: &str) -> Result<StoredObject> {
        let mut conn = self.lock_writer()?;
        let tx = conn.transaction()?;

        let parent_id: Option<String> = tx
            .query_row(
                "SELECT parent_id FROM objects WHERE id = ?1 AND deleted = 0",
                params![id.0],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| MetadataError::NotFound(id.0.clone()))?;

        let parent_file_id = parent_id.as_deref().map(|s| FileId(s.to_string()));
        let sanitized_name = new_name.replace(['/', '\\'], "_");
        let local_name =
            Self::resolve_unique_name(&tx, parent_file_id.as_ref(), &sanitized_name, id)?;

        tx.execute(
            "UPDATE objects SET name = ?1, remote_name = ?2 WHERE id = ?3",
            params![local_name, new_name, id.0],
        )?;
        tx.commit()?;
        drop(conn);

        self.get_object(id)?
            .ok_or_else(|| MetadataError::NotFound(id.0.clone()))
    }

    pub fn move_object(
        &self,
        id: &FileId,
        new_parent_id: Option<&FileId>,
        new_name: &str,
    ) -> Result<StoredObject> {
        let mut conn = self.lock_writer()?;
        let tx = conn.transaction()?;

        let sanitized_name = new_name.replace(['/', '\\'], "_");
        let local_name = Self::resolve_unique_name(&tx, new_parent_id, &sanitized_name, id)?;
        let parent_str = new_parent_id.map(|p| p.0.as_str());

        tx.execute(
            "UPDATE objects SET parent_id = ?1, name = ?2, remote_name = ?3 WHERE id = ?4 AND deleted = 0",
            params![parent_str, local_name, new_name, id.0],
        )?;
        tx.commit()?;
        drop(conn);

        self.get_object(id)?
            .ok_or_else(|| MetadataError::NotFound(id.0.clone()))
    }

    pub fn update_size_and_mtime(
        &self,
        id: &FileId,
        size: u64,
        modified_at: Option<&str>,
    ) -> Result<()> {
        self.update_size_mtime_version(id, size, modified_at, None)
    }

    pub fn update_size_mtime_version(
        &self,
        id: &FileId,
        size: u64,
        modified_at: Option<&str>,
        version: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock_writer()?;
        conn.execute(
            "UPDATE objects SET size_bytes = ?1, modified_at = ?2, version = COALESCE(?3, version) WHERE id = ?4 AND deleted = 0",
            params![size as i64, modified_at, version, id.0],
        )?;
        Ok(())
    }

    pub fn is_directory_synced(&self, dir_id: Option<&FileId>) -> Result<bool> {
        let conn = self.lock_reader()?;
        let id_str = dir_id.map(|p| p.0.as_str()).unwrap_or("__root__");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM synced_directories WHERE dir_id = ?1",
            params![id_str],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn mark_directory_synced(&self, dir_id: Option<&FileId>) -> Result<()> {
        let conn = self.lock_writer()?;
        let id_str = dir_id.map(|p| p.0.as_str()).unwrap_or("__root__");
        conn.execute(
            "INSERT INTO synced_directories (dir_id, synced_at) VALUES (?1, datetime('now'))
             ON CONFLICT(dir_id) DO UPDATE SET synced_at = excluded.synced_at",
            params![id_str],
        )?;
        Ok(())
    }

    /// Marks an object as deleted; semantically represents provider-side trash.
    pub fn mark_trashed(&self, id: &FileId) -> Result<()> {
        let conn = self.lock_writer()?;
        conn.execute(
            "UPDATE objects SET deleted = 1 WHERE id = ?1",
            params![id.0],
        )?;
        Ok(())
    }

    pub fn insert_new_object(&self, obj: &StoredObject) -> Result<()> {
        let conn = self.lock_writer()?;
        let parent_str = obj.parent_id.as_ref().map(|p| p.0.as_str());
        let kind_str = match obj.kind {
            ObjectKind::Directory => "directory",
            ObjectKind::File => "file",
        };
        let size_val = obj.size_bytes.map(|s| s as i64);

        conn.execute(
            "INSERT INTO objects (
                id, parent_id, name, remote_name, kind, size_bytes,
                mime_type, created_at, modified_at, version, sync_status, deleted
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
            params![
                obj.id.0,
                parent_str,
                obj.name,
                obj.remote_name,
                kind_str,
                size_val,
                obj.mime_type,
                obj.created_at,
                obj.modified_at,
                obj.version,
                obj.sync_status.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn has_children(&self, parent_id: &FileId) -> Result<bool> {
        let conn = self.lock_reader()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM objects WHERE parent_id = ?1 AND deleted = 0",
            params![parent_id.0],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn is_descendant_of(
        &self,
        potential_descendant_id: &FileId,
        ancestor_id: &FileId,
    ) -> Result<bool> {
        if potential_descendant_id == ancestor_id {
            return Ok(true);
        }

        let conn = self.lock_reader()?;
        let mut current_id = Some(potential_descendant_id.clone());

        while let Some(curr) = current_id {
            let parent: Option<Option<String>> = conn
                .query_row(
                    "SELECT parent_id FROM objects WHERE id = ?1 AND deleted = 0",
                    params![curr.0],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()?;

            match parent {
                Some(Some(ref p)) if p == &ancestor_id.0 => return Ok(true),
                Some(Some(p)) => current_id = Some(FileId(p)),
                _ => break,
            }
        }

        Ok(false)
    }

    pub fn replace_file_id(&self, old_id: &FileId, new_meta: &StoredObject) -> Result<()> {
        let mut conn = self.lock_writer()?;
        let tx = conn.transaction()?;

        let parent_str = new_meta.parent_id.as_ref().map(|p| p.0.as_str());
        let kind_str = match new_meta.kind {
            ObjectKind::Directory => "directory",
            ObjectKind::File => "file",
        };
        let size_val = new_meta.size_bytes.map(|s| s as i64);

        tx.execute(
            "UPDATE objects SET parent_id = ?1 WHERE parent_id = ?2",
            params![new_meta.id.0, old_id.0],
        )?;

        let updated = tx.execute(
            "UPDATE objects SET 
                id = ?1, parent_id = ?2, name = ?3, remote_name = ?4, kind = ?5,
                size_bytes = ?6, mime_type = ?7, created_at = ?8, modified_at = ?9,
                version = ?10, sync_status = ?11, deleted = 0
             WHERE id = ?12",
            params![
                new_meta.id.0,
                parent_str,
                new_meta.name,
                new_meta.remote_name,
                kind_str,
                size_val,
                new_meta.mime_type,
                new_meta.created_at,
                new_meta.modified_at,
                new_meta.version,
                new_meta.sync_status.as_str(),
                old_id.0,
            ],
        )?;

        if updated == 0 {
            tx.execute(
                "INSERT INTO objects (
                    id, parent_id, name, remote_name, kind, size_bytes,
                    mime_type, created_at, modified_at, version, sync_status, deleted
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)
                ON CONFLICT(id) DO UPDATE SET
                    parent_id = excluded.parent_id,
                    name = excluded.name,
                    remote_name = excluded.remote_name,
                    kind = excluded.kind,
                    size_bytes = excluded.size_bytes,
                    mime_type = excluded.mime_type,
                    created_at = excluded.created_at,
                    modified_at = excluded.modified_at,
                    version = excluded.version,
                    sync_status = excluded.sync_status,
                    deleted = 0",
                params![
                    new_meta.id.0,
                    parent_str,
                    new_meta.name,
                    new_meta.remote_name,
                    kind_str,
                    size_val,
                    new_meta.mime_type,
                    new_meta.created_at,
                    new_meta.modified_at,
                    new_meta.version,
                    new_meta.sync_status.as_str(),
                ],
            )?;
        }

        tx.execute(
            "UPDATE staging_journal SET file_id = ?1 WHERE file_id = ?2",
            params![new_meta.id.0, old_id.0],
        )?;
        tx.execute(
            "UPDATE staging_journal SET parent_id = ?1 WHERE parent_id = ?2",
            params![new_meta.id.0, old_id.0],
        )?;

        tx.execute(
            "UPDATE synced_directories SET dir_id = ?1 WHERE dir_id = ?2",
            params![new_meta.id.0, old_id.0],
        )?;

        tx.commit()?;
        Ok(())
    }

    pub fn record_staging_entry(
        &self,
        handle_id: u64,
        file_id: &FileId,
        staging_path: &str,
        parent_id: Option<&FileId>,
    ) -> Result<()> {
        let conn = self.lock_writer()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO staging_journal 
                (handle_id, file_id, staging_path, parent_id, state, session_uri, uploaded_bytes, created_at, updated_at, direction, error_count)
             VALUES (?1, ?2, ?3, ?4, 'staging', NULL, 0, ?5, ?5, 'upload', 0)",
            params![
                handle_id as i64,
                file_id.0,
                staging_path,
                parent_id.map(|p| p.0.as_str()),
                now,
            ],
        )?;

        Ok(())
    }

    pub fn enqueue_delete(&self, file_id: &FileId, handle_id: Option<u64>) -> Result<()> {
        let conn = self.lock_writer()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let h_id = handle_id.unwrap_or_else(|| {
            use std::sync::atomic::{AtomicU64, Ordering};
            static DEL_COUNTER: AtomicU64 = AtomicU64::new(0x7000_0000_0000_0000);
            DEL_COUNTER.fetch_add(1, Ordering::Relaxed)
        });

        conn.execute(
            "INSERT OR REPLACE INTO staging_journal 
                (handle_id, file_id, staging_path, parent_id, state, session_uri, uploaded_bytes, created_at, updated_at, direction, error_count)
             VALUES (?1, ?2, '', NULL, 'staging', NULL, 0, ?3, ?3, 'delete', 0)",
            params![h_id as i64, file_id.0, now],
        )?;

        Ok(())
    }

    pub fn increment_staging_error(&self, handle_id: u64) -> Result<u32> {
        let conn = self.lock_writer()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "UPDATE staging_journal SET error_count = error_count + 1, updated_at = ?1 WHERE handle_id = ?2",
            params![now, handle_id as i64],
        )?;

        let count: i64 = conn.query_row(
            "SELECT error_count FROM staging_journal WHERE handle_id = ?1",
            params![handle_id as i64],
            |r| r.get(0),
        )?;

        Ok(count as u32)
    }

    pub fn get_pending_sync_stats(&self) -> Result<(usize, u64)> {
        let conn = self.lock_reader()?;
        let (count, bytes): (i64, Option<i64>) = conn.query_row(
            "SELECT COUNT(*), SUM(COALESCE(o.size_bytes, 0)) 
             FROM staging_journal j
             LEFT JOIN objects o ON j.file_id = o.id
             WHERE j.state != 'failed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok((count as usize, bytes.unwrap_or(0).max(0) as u64))
    }

    pub fn update_staging_state(&self, handle_id: u64, state: StagingState) -> Result<()> {
        let conn = self.lock_writer()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "UPDATE staging_journal SET state = ?1, updated_at = ?2 WHERE handle_id = ?3",
            params![state.as_str(), now, handle_id as i64],
        )?;

        Ok(())
    }

    pub fn update_staging_session(
        &self,
        handle_id: u64,
        session_uri: &str,
        uploaded_bytes: u64,
    ) -> Result<()> {
        let conn = self.lock_writer()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "UPDATE staging_journal 
             SET session_uri = ?1, uploaded_bytes = ?2, updated_at = ?3 
             WHERE handle_id = ?4",
            params![session_uri, uploaded_bytes as i64, now, handle_id as i64],
        )?;

        Ok(())
    }

    pub fn remove_staging_entry(&self, handle_id: u64) -> Result<()> {
        let conn = self.lock_writer()?;
        conn.execute(
            "DELETE FROM staging_journal WHERE handle_id = ?1",
            params![handle_id as i64],
        )?;
        Ok(())
    }

    pub fn list_uncommitted_staging(&self) -> Result<Vec<StagingJournalEntry>> {
        let conn = self.lock_reader()?;
        let mut stmt = conn.prepare_cached(
            "SELECT handle_id, file_id, staging_path, parent_id, state, session_uri, uploaded_bytes, created_at, updated_at, direction, error_count
             FROM staging_journal
             ORDER BY created_at ASC, rowid ASC",
        )?;

        let entries = stmt
            .query_map([], |row| {
                let handle_id: i64 = row.get(0)?;
                let file_id: String = row.get(1)?;
                let staging_path: String = row.get(2)?;
                let parent_id: Option<String> = row.get(3)?;
                let state_str: String = row.get(4)?;
                let session_uri: Option<String> = row.get(5)?;
                let uploaded_bytes: Option<i64> = row.get(6)?;
                let created_at: i64 = row.get(7)?;
                let updated_at: i64 = row.get(8)?;
                let direction: String = row.get(9)?;
                let error_count: i64 = row.get(10)?;

                Ok(StagingJournalEntry {
                    handle_id: handle_id as u64,
                    file_id: FileId(file_id),
                    staging_path,
                    parent_id: parent_id.map(FileId),
                    state: state_str.parse().unwrap_or(StagingState::Staging),
                    session_uri,
                    uploaded_bytes: uploaded_bytes.unwrap_or(0) as u64,
                    created_at,
                    updated_at,
                    direction,
                    error_count: error_count as u32,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }
}

fn split_name_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(idx) if idx > 0 => (&name[..idx], &name[idx..]),
        _ => (name, ""),
    }
}

fn row_to_stored_object(row: &Row) -> rusqlite::Result<StoredObject> {
    let id: String = row.get(0)?;
    let parent_id: Option<String> = row.get(1)?;
    let name: String = row.get(2)?;
    let remote_name: String = row.get(3)?;
    let kind_str: String = row.get(4)?;
    let size_bytes: Option<i64> = row.get(5)?;
    let mime_type: Option<String> = row.get(6)?;
    let created_at: Option<String> = row.get(7)?;
    let modified_at: Option<String> = row.get(8)?;
    let version: Option<String> = row.get(9)?;
    let status_str: String = row.get(10)?;
    let deleted_int: i64 = row.get(11)?;

    let kind = match kind_str.as_str() {
        "directory" => ObjectKind::Directory,
        _ => ObjectKind::File,
    };

    Ok(StoredObject {
        id: FileId(id),
        parent_id: parent_id.map(FileId),
        name,
        remote_name,
        kind,
        size_bytes: size_bytes.map(|s| s as u64),
        mime_type,
        created_at,
        modified_at,
        version,
        sync_status: status_str.parse().unwrap_or(SyncStatus::Synced),
        deleted: deleted_int != 0,
    })
}

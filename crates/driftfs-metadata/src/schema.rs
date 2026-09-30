use crate::error::{MetadataError, Result};
use rusqlite::Connection;

const CURRENT_SCHEMA_VERSION: u32 = 4;

pub fn run_migrations(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );",
    )?;

    let current_version: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if current_version < 1 {
        apply_v1(conn)?;
    }
    if current_version < 2 {
        apply_v2(conn)?;
    }
    if current_version < 3 {
        apply_v3(conn)?;
    }
    if current_version < CURRENT_SCHEMA_VERSION {
        apply_v4(conn)?;
    }

    Ok(())
}

fn apply_v1(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS objects (
            id TEXT PRIMARY KEY,
            parent_id TEXT,
            name TEXT NOT NULL COLLATE NOCASE,
            remote_name TEXT NOT NULL,
            kind TEXT NOT NULL,
            size_bytes INTEGER,
            mime_type TEXT,
            created_at TEXT,
            modified_at TEXT,
            version TEXT,
            sync_status TEXT NOT NULL DEFAULT 'synced',
            deleted INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_objects_parent 
            ON objects (parent_id, deleted);

        CREATE INDEX IF NOT EXISTS idx_objects_lookup 
            ON objects (parent_id, name COLLATE NOCASE, deleted);

        CREATE INDEX IF NOT EXISTS idx_objects_deleted 
            ON objects (deleted);

        CREATE TABLE IF NOT EXISTS sync_checkpoints (
            account_id TEXT PRIMARY KEY,
            checkpoint_token TEXT NOT NULL,
            last_sync_at TEXT NOT NULL
        );

        INSERT INTO schema_migrations (version, applied_at)
        VALUES (1, datetime('now'));",
    )
    .map_err(|e| MetadataError::MigrationFailed {
        version: 1,
        reason: e.to_string(),
    })?;

    tx.commit()?;
    Ok(())
}

fn apply_v2(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS synced_directories (
            dir_id TEXT PRIMARY KEY,
            synced_at TEXT NOT NULL
        );

        INSERT INTO schema_migrations (version, applied_at)
        VALUES (2, datetime('now'));",
    )
    .map_err(|e| MetadataError::MigrationFailed {
        version: 2,
        reason: e.to_string(),
    })?;

    tx.commit()?;
    Ok(())
}

fn apply_v3(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS staging_journal (
            handle_id INTEGER PRIMARY KEY,
            file_id TEXT NOT NULL,
            staging_path TEXT NOT NULL,
            parent_id TEXT,
            state TEXT NOT NULL DEFAULT 'staging',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_staging_journal_file_id 
            ON staging_journal (file_id);

        INSERT INTO schema_migrations (version, applied_at)
        VALUES (3, datetime('now'));",
    )
    .map_err(|e| MetadataError::MigrationFailed {
        version: 3,
        reason: e.to_string(),
    })?;

    tx.commit()?;
    Ok(())
}

fn apply_v4(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_objects_parent_sorted 
            ON objects (parent_id, deleted, kind DESC, name COLLATE NOCASE ASC);

        ALTER TABLE staging_journal ADD COLUMN session_uri TEXT;
        ALTER TABLE staging_journal ADD COLUMN uploaded_bytes INTEGER DEFAULT 0;

        INSERT INTO schema_migrations (version, applied_at)
        VALUES (4, datetime('now'));",
    )
    .map_err(|e| MetadataError::MigrationFailed {
        version: 4,
        reason: e.to_string(),
    })?;

    tx.commit()?;
    Ok(())
}

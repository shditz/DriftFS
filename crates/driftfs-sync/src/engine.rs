use std::sync::Arc;

use driftfs_cache::BoundedChunkCache;
use driftfs_core::{AccountId, DriftFsError, FileId};
use driftfs_metadata::{MetadataStore, StoredObject};
use driftfs_provider::CloudProvider;

use crate::error::{Result, SyncError};

pub struct SyncEngine<P: CloudProvider> {
    provider: Arc<P>,
    store: Arc<MetadataStore>,
    account_id: AccountId,
    root_id: FileId,
    chunk_cache: Option<Arc<BoundedChunkCache>>,
}

impl<P: CloudProvider> SyncEngine<P> {
    pub fn new(provider: Arc<P>, store: Arc<MetadataStore>, account_id: AccountId) -> Self {
        Self {
            provider,
            store,
            account_id,
            root_id: FileId("root".into()),
            chunk_cache: None,
        }
    }

    pub fn with_root_id(mut self, root_id: FileId) -> Self {
        self.root_id = root_id;
        self
    }

    pub fn with_chunk_cache(mut self, chunk_cache: Arc<BoundedChunkCache>) -> Self {
        self.chunk_cache = Some(chunk_cache);
        self
    }

    pub fn root_id(&self) -> &FileId {
        &self.root_id
    }

    pub fn store(&self) -> &Arc<MetadataStore> {
        &self.store
    }

    pub fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    fn is_root_parent(&self, parent_id: Option<&FileId>) -> bool {
        match parent_id {
            Some(p) => {
                p == &self.root_id
                    || p.0 == "root"
                    || !self
                        .store
                        .get_object(p)
                        .map(|opt| opt.is_some())
                        .unwrap_or(true)
            }
            None => true,
        }
    }

    fn normalize_parent_id(
        &self,
        mut meta: driftfs_provider::ObjectMetadata,
    ) -> driftfs_provider::ObjectMetadata {
        if self.is_root_parent(meta.parent_id.as_ref()) {
            meta.parent_id = None;
        }
        meta
    }

    pub async fn bootstrap_root(&self) -> Result<Vec<StoredObject>> {
        let mut children = self.provider.list_children(&self.root_id).await?;
        for child in &mut children {
            child.parent_id = None;
        }

        if !children.is_empty() {
            self.store.batch_upsert_objects(&children)?;
        }

        if self.store.get_checkpoint(&self.account_id)?.is_none() {
            let initial_page = self.provider.fetch_changes(None).await?;
            if let Some(token) = initial_page.next_checkpoint {
                self.store.set_checkpoint(&self.account_id, &token)?;
            }
        }

        let _ = self.store.mark_directory_synced(None);
        let _ = self.store.normalize_root_orphans();
        let stored = self.store.list_children(None)?;
        Ok(stored)
    }

    pub async fn sync_changes(&self) -> Result<usize> {
        let current_checkpoint = self.store.get_checkpoint(&self.account_id)?;

        let checkpoint_token = match current_checkpoint {
            Some(token) => token,
            None => {
                let objects = self.bootstrap_root().await?;
                return Ok(objects.len());
            }
        };

        let fetch_res = self.provider.fetch_changes(Some(&checkpoint_token)).await;

        let page = match fetch_res {
            Ok(p) => p,
            Err(e) => {
                if matches!(e, DriftFsError::SyncCheckpointExpired { .. })
                    || e.to_string().contains("410")
                    || e.to_string().contains("expired")
                {
                    tracing::warn!(?e, "sync checkpoint expired, triggering reconciliation");
                    self.reconcile().await?;
                    return Ok(0);
                }
                return Err(SyncError::Provider(e));
            }
        };

        let count = page.changes.len();

        let normalized_changes: Vec<driftfs_provider::Change> = page
            .changes
            .into_iter()
            .map(|c| match c {
                driftfs_provider::Change::Upsert(meta) => {
                    driftfs_provider::Change::Upsert(self.normalize_parent_id(meta))
                }
                other => other,
            })
            .collect();

        self.store.batch_apply_changes(
            &normalized_changes,
            page.next_checkpoint.as_deref(),
            &self.account_id,
        )?;

        if let Some(ref cache) = self.chunk_cache {
            for c in &normalized_changes {
                match c {
                    driftfs_provider::Change::Upsert(meta) => {
                        let _ = cache.invalidate_file(&meta.id).await;
                    }
                    driftfs_provider::Change::Delete { id } => {
                        let _ = cache.invalidate_file(id).await;
                    }
                }
            }
        }

        Ok(count)
    }

    pub async fn reconcile(&self) -> Result<()> {
        let initial_page = self.provider.fetch_changes(None).await?;
        let mut children = self.provider.list_children(&self.root_id).await?;
        for child in &mut children {
            if self.is_root_parent(child.parent_id.as_ref()) {
                child.parent_id = None;
            }
        }

        if !children.is_empty() {
            self.store.batch_upsert_objects(&children)?;
        }

        if let Some(token) = initial_page.next_checkpoint {
            self.store.set_checkpoint(&self.account_id, &token)?;
        }

        Ok(())
    }

    pub async fn process_outbound_queue(&self) -> Result<usize> {
        let entries = self.store.list_uncommitted_staging()?;
        if entries.is_empty() {
            return Ok(0);
        }

        let mut processed = 0;

        for entry in entries {
            if entry.state == driftfs_metadata::StagingState::Failed && entry.error_count >= 5 {
                continue;
            }

            if entry.direction == "delete" {
                match self.provider.trash(&entry.file_id).await {
                    Ok(_) => {
                        let _ = self.store.remove_staging_entry(entry.handle_id);
                        processed += 1;
                    }
                    Err(e) => {
                        if matches!(e, DriftFsError::NotFound { .. })
                            || e.to_string().contains("404")
                            || e.to_string().contains("not found")
                        {
                            let _ = self.store.remove_staging_entry(entry.handle_id);
                            processed += 1;
                        } else {
                            tracing::warn!(
                                handle = entry.handle_id,
                                file_id = %entry.file_id,
                                ?e,
                                "outbound delete failed"
                            );
                            let err_count = self
                                .store
                                .increment_staging_error(entry.handle_id)
                                .unwrap_or(0);
                            if err_count >= 5 {
                                let _ = self.store.update_staging_state(
                                    entry.handle_id,
                                    driftfs_metadata::StagingState::Failed,
                                );
                            }
                        }
                    }
                }
            } else {
                let obj = self.store.get_object(&entry.file_id)?;
                let (obj_name, obj_kind, parent_id) = match obj {
                    Some(ref o) => (o.name.clone(), o.kind, o.parent_id.clone()),
                    None => {
                        let _ = self.store.remove_staging_entry(entry.handle_id);
                        continue;
                    }
                };

                let effective_parent = match parent_id {
                    Some(ref p) => {
                        if p.is_local() {
                            continue;
                        }
                        p.clone()
                    }
                    None => self.root_id.clone(),
                };

                if obj_kind == driftfs_provider::ObjectKind::Directory {
                    match self
                        .provider
                        .create_directory(&effective_parent, &obj_name)
                        .await
                    {
                        Ok(remote_meta) => {
                            let stored = StoredObject {
                                id: remote_meta.id.clone(),
                                parent_id: parent_id.clone(),
                                name: obj_name.clone(),
                                remote_name: obj_name,
                                kind: driftfs_provider::ObjectKind::Directory,
                                size_bytes: None,
                                mime_type: remote_meta.mime_type,
                                created_at: remote_meta.created_at,
                                modified_at: remote_meta.modified_at,
                                version: remote_meta.version,
                                sync_status: driftfs_metadata::SyncStatus::Synced,
                                deleted: false,
                            };
                            let _ = self.store.replace_file_id(&entry.file_id, &stored);
                            let _ = self.store.remove_staging_entry(entry.handle_id);
                            processed += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                handle = entry.handle_id,
                                dir_id = %entry.file_id,
                                ?e,
                                "outbound mkdir failed"
                            );
                            let err_count = self
                                .store
                                .increment_staging_error(entry.handle_id)
                                .unwrap_or(0);
                            if err_count >= 5 {
                                let _ = self.store.update_staging_state(
                                    entry.handle_id,
                                    driftfs_metadata::StagingState::Failed,
                                );
                            }
                        }
                    }
                } else {
                    let staging_path = std::path::PathBuf::from(&entry.staging_path);
                    if !staging_path.exists() {
                        let _ = self.store.remove_staging_entry(entry.handle_id);
                        continue;
                    }

                    let _ = self.store.update_staging_state(
                        entry.handle_id,
                        driftfs_metadata::StagingState::Uploading,
                    );

                    let store_clone = Arc::clone(&self.store);
                    let handle_id = entry.handle_id;
                    let on_session = Arc::new(move |session_uri: &str| {
                        let _ = store_clone.update_staging_session(handle_id, session_uri, 0);
                    });

                    let upload_res = if entry.file_id.is_local() {
                        match self
                            .provider
                            .create_file(&effective_parent, &obj_name)
                            .await
                        {
                            Ok(created_meta) => {
                                self.provider
                                    .upload_file_resumable(
                                        &created_meta.id,
                                        &staging_path,
                                        entry.session_uri.as_deref(),
                                        Some(on_session),
                                    )
                                    .await
                            }
                            Err(e) => Err(e),
                        }
                    } else {
                        self.provider
                            .upload_file_resumable(
                                &entry.file_id,
                                &staging_path,
                                entry.session_uri.as_deref(),
                                Some(on_session),
                            )
                            .await
                    };

                    match upload_res {
                        Ok(updated_meta) => {
                            let new_size = updated_meta.size_bytes.unwrap_or(0);
                            if entry.file_id.is_local() {
                                let stored = StoredObject {
                                    id: updated_meta.id.clone(),
                                    parent_id: parent_id.clone(),
                                    name: obj_name.clone(),
                                    remote_name: obj_name,
                                    kind: driftfs_provider::ObjectKind::File,
                                    size_bytes: Some(new_size),
                                    mime_type: updated_meta.mime_type,
                                    created_at: updated_meta.created_at,
                                    modified_at: updated_meta.modified_at.clone(),
                                    version: updated_meta.version.clone(),
                                    sync_status: driftfs_metadata::SyncStatus::Synced,
                                    deleted: false,
                                };
                                let _ = self.store.replace_file_id(&entry.file_id, &stored);
                            } else {
                                let _ = self.store.update_size_mtime_version(
                                    &entry.file_id,
                                    new_size,
                                    updated_meta.modified_at.as_deref(),
                                    updated_meta.version.as_deref(),
                                );
                            }

                            let _ = self.store.remove_staging_entry(entry.handle_id);
                            let _ = std::fs::remove_file(&staging_path);

                            if let Some(ref cache) = self.chunk_cache {
                                let _ = cache.invalidate_file(&updated_meta.id).await;
                            }
                            processed += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                handle = entry.handle_id,
                                file_id = %entry.file_id,
                                ?e,
                                "outbound file upload failed"
                            );
                            let err_count = self
                                .store
                                .increment_staging_error(entry.handle_id)
                                .unwrap_or(0);
                            if err_count >= 5 {
                                let _ = self.store.update_staging_state(
                                    entry.handle_id,
                                    driftfs_metadata::StagingState::Failed,
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(processed)
    }
}

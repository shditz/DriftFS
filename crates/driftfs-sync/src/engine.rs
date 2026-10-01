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
}

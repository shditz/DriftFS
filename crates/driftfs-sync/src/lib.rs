pub mod engine;
pub mod error;
pub mod worker;

pub use engine::SyncEngine;
pub use error::{Result, SyncError};
pub use worker::SyncWorker;

#[cfg(test)]
mod tests {
    use super::*;
    use driftfs_core::{AccountId, FileId};
    use driftfs_metadata::MetadataStore;
    use driftfs_provider::{Change, ChangePage, ObjectKind, ObjectMetadata};
    use driftfs_testkit::MockProvider;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn bootstrap_root_populates_store_and_checkpoint() {
        let mock = MockProvider::new();
        mock.add_directory("root", "My Drive", None);
        mock.add_file("f1", "notes.txt", Some("root"), 512);
        mock.add_file("f2", "photo.jpg", Some("root"), 2048);
        mock.add_directory("d1", "Documents", Some("root"));

        let provider = Arc::new(mock);
        let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
        let account = AccountId("test_acc".into());

        let engine = SyncEngine::new(provider, store.clone(), account.clone());
        let root_items = engine.bootstrap_root().await.expect("bootstrap");

        assert_eq!(root_items.len(), 3);
        assert!(store
            .get_object(&FileId("f1".into()))
            .expect("get")
            .is_some());
        assert!(store
            .get_object(&FileId("f2".into()))
            .expect("get")
            .is_some());
        assert!(store
            .get_object(&FileId("d1".into()))
            .expect("get")
            .is_some());
    }

    #[tokio::test]
    async fn incremental_sync_applies_changes_atomically() {
        let mock = MockProvider::new();
        let provider = Arc::new(mock);
        let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
        let account = AccountId("test_acc".into());

        let engine = SyncEngine::new(provider.clone(), store.clone(), account.clone());

        store.set_checkpoint(&account, "cp_v1").expect("set cp");
        store
            .upsert_object(&ObjectMetadata {
                id: FileId("old_f".into()),
                name: "old.txt".into(),
                parent_id: Some(FileId("root".into())),
                kind: ObjectKind::File,
                size_bytes: Some(100),
                mime_type: Some("text/plain".into()),
                created_at: None,
                modified_at: None,
                version: Some("v1".into()),
            })
            .expect("upsert");

        let page = ChangePage {
            changes: vec![
                Change::Delete {
                    id: FileId("old_f".into()),
                },
                Change::Upsert(ObjectMetadata {
                    id: FileId("new_f".into()),
                    name: "new.txt".into(),
                    parent_id: Some(FileId("root".into())),
                    kind: ObjectKind::File,
                    size_bytes: Some(250),
                    mime_type: Some("text/plain".into()),
                    created_at: None,
                    modified_at: None,
                    version: Some("v2".into()),
                }),
            ],
            next_checkpoint: Some("cp_v2".into()),
        };
        provider.enqueue_change_page(page);

        let applied = engine.sync_changes().await.expect("sync");
        assert_eq!(applied, 2);

        assert!(store
            .get_object(&FileId("old_f".into()))
            .expect("get")
            .is_none());
        assert!(store
            .get_object(&FileId("new_f".into()))
            .expect("get")
            .is_some());

        let current_cp = store.get_checkpoint(&account).expect("cp");
        assert_eq!(current_cp.as_deref(), Some("cp_v2"));
    }

    #[tokio::test]
    async fn sync_worker_cancellation_exits_cleanly() {
        let mock = MockProvider::new();
        let provider = Arc::new(mock);
        let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
        let account = AccountId("test_acc".into());

        let engine = Arc::new(SyncEngine::new(provider, store, account));
        let cancel = CancellationToken::new();

        let worker = SyncWorker::new(engine, Duration::from_millis(50), cancel.clone());

        let handle = tokio::spawn(async move {
            worker.run().await;
        });

        tokio::time::sleep(Duration::from_millis(120)).await;
        cancel.cancel();

        let res = tokio::time::timeout(Duration::from_secs(1), handle).await;
        assert!(res.is_ok(), "worker should terminate after cancellation");
    }

    #[tokio::test]
    async fn sync_changes_invalidates_chunk_cache() {
        let mock = MockProvider::new();
        let provider = Arc::new(mock);
        let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
        let account = AccountId("test_acc".into());
        store.set_checkpoint(&account, "cp_v1").expect("set cp");

        let cache_dir = tempfile::tempdir().expect("cache dir");
        let cache = Arc::new(
            driftfs_cache::BoundedChunkCache::new(
                cache_dir.path().to_path_buf(),
                10 * 1024 * 1024,
                driftfs_cache::DEFAULT_CHUNK_SIZE,
            )
            .expect("cache open"),
        );

        let file_id = FileId("cached_f".into());
        let key = driftfs_cache::ChunkKey::new(file_id.clone(), "v1", 0);
        cache
            .put(key.clone(), &[1, 2, 3, 4])
            .await
            .expect("put chunk");
        assert!(cache.get(&key).await.unwrap().is_some());

        let engine = SyncEngine::new(provider.clone(), store.clone(), account.clone())
            .with_chunk_cache(cache.clone());

        let page = ChangePage {
            changes: vec![Change::Delete { id: file_id }],
            next_checkpoint: Some("cp_after_del".into()),
        };
        provider.enqueue_change_page(page);

        let count = engine.sync_changes().await.expect("sync");
        assert_eq!(count, 1);
        assert!(cache.get(&key).await.unwrap().is_none());
    }
}

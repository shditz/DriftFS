use std::sync::Arc;
use std::time::Duration;

use driftfs_core::{AccountId, FileId};
use driftfs_metadata::MetadataStore;
use driftfs_provider::{Change, ChangePage, ObjectKind, ObjectMetadata};
use driftfs_sync::SyncEngine;
use driftfs_testkit::MockProvider;

fn create_test_obj(id: &str, name: &str, parent: Option<&str>) -> ObjectMetadata {
    ObjectMetadata {
        id: FileId(id.into()),
        name: name.into(),
        parent_id: parent.map(|p| FileId(p.into())),
        kind: ObjectKind::File,
        size_bytes: Some(1024),
        mime_type: Some("text/plain".into()),
        created_at: None,
        modified_at: None,
        version: Some("1".into()),
    }
}

#[tokio::test]
async fn concurrent_sync_and_metadata_queries_under_wal() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let db_path = temp_dir.path().join("metadata.db");

    let store = Arc::new(MetadataStore::open(&db_path).expect("open store"));
    let provider = Arc::new(MockProvider::new());
    let account = AccountId("concurrent_user@driftfs.local".into());

    let engine = Arc::new(SyncEngine::new(
        Arc::clone(&provider),
        Arc::clone(&store),
        account.clone(),
    ));

    // Seed initial objects
    let initial_objs = vec![
        create_test_obj("init_1", "init_1.txt", None),
        create_test_obj("init_2", "init_2.txt", None),
    ];
    store
        .batch_upsert_objects(&initial_objs)
        .expect("batch upsert");
    store.set_checkpoint(&account, "cp_0").expect("set cp");

    let mut handles = Vec::new();

    // Task 1: Continuous sync worker applying remote change pages
    let engine_c = Arc::clone(&engine);
    let provider_c = Arc::clone(&provider);
    handles.push(tokio::spawn(async move {
        for i in 1..=20 {
            let page = ChangePage {
                changes: vec![Change::Upsert(create_test_obj(
                    &format!("sync_file_{i}"),
                    &format!("remote_{i}.txt"),
                    None,
                ))],
                next_checkpoint: Some(format!("cp_{i}")),
            };
            provider_c.enqueue_change_page(page);

            let res = engine_c.sync_changes().await;
            assert!(res.is_ok(), "sync_changes failed: {:?}", res);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }));

    // Task 2: Continuous local reader querying objects and hierarchy
    let store_c1 = Arc::clone(&store);
    handles.push(tokio::spawn(async move {
        for _ in 0..50 {
            let _ = store_c1.lookup_by_name(None, "init_1.txt").expect("lookup");
            let _ = store_c1.list_children(None).expect("list");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }));

    // Task 3: Continuous local writer updating sizes and adding local staging
    let store_c2 = Arc::clone(&store);
    handles.push(tokio::spawn(async move {
        for i in 0..25 {
            let local_obj =
                create_test_obj(&format!("local_file_{i}"), &format!("local_{i}.txt"), None);
            store_c2.upsert_object(&local_obj).expect("upsert local");
            store_c2
                .update_size_mtime_version(
                    &FileId(format!("local_file_{i}")),
                    (i * 100) as u64,
                    None,
                    None,
                )
                .expect("update size");
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
    }));

    for h in handles {
        h.await.expect("concurrency task failed");
    }

    // Verify all sync changes applied and checkpoint advanced
    let latest_cp = store.get_checkpoint(&account).expect("get cp");
    assert_eq!(latest_cp.as_deref(), Some("cp_20"));

    // Verify both remote and local files exist in store
    assert!(store
        .get_object(&FileId("sync_file_20".into()))
        .unwrap()
        .is_some());
    assert!(store
        .get_object(&FileId("local_file_24".into()))
        .unwrap()
        .is_some());
}

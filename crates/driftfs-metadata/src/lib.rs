pub mod error;
pub mod model;
pub mod schema;
pub mod store;

pub use error::{MetadataError, Result};
pub use model::{StagingJournalEntry, StagingState, StoredObject, SyncStatus};
pub use store::MetadataStore;

#[cfg(test)]
mod tests {
    use super::*;
    use driftfs_core::{AccountId, FileId};
    use driftfs_provider::{Change, ObjectKind, ObjectMetadata};

    fn make_test_meta(
        id: &str,
        parent: Option<&str>,
        name: &str,
        kind: ObjectKind,
    ) -> ObjectMetadata {
        ObjectMetadata {
            id: FileId(id.into()),
            name: name.into(),
            parent_id: parent.map(|p| FileId(p.into())),
            kind,
            size_bytes: if kind == ObjectKind::File {
                Some(1024)
            } else {
                None
            },
            mime_type: if kind == ObjectKind::File {
                Some("text/plain".into())
            } else {
                Some("application/vnd.google-apps.folder".into())
            },
            created_at: Some("2026-09-01T00:00:00Z".into()),
            modified_at: Some("2026-09-01T00:00:00Z".into()),
            version: Some("v1".into()),
        }
    }

    #[test]
    fn opens_in_memory_and_migrates() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let root_children = store.list_children(None).expect("list children");
        assert!(root_children.is_empty());
    }

    #[test]
    fn upsert_and_retrieve_object() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let meta = make_test_meta("file_1", None, "notes.txt", ObjectKind::File);
        store.upsert_object(&meta).expect("upsert");

        let retrieved = store.get_object(&meta.id).expect("get").expect("found");
        assert_eq!(retrieved.id, meta.id);
        assert_eq!(retrieved.name, "notes.txt");
        assert_eq!(retrieved.remote_name, "notes.txt");
        assert_eq!(retrieved.kind, ObjectKind::File);
        assert_eq!(retrieved.size_bytes, Some(1024));
        assert_eq!(retrieved.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn hierarchical_child_listing() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let folder = make_test_meta("folder_1", None, "Documents", ObjectKind::Directory);
        let child1 = make_test_meta("file_1", Some("folder_1"), "a.txt", ObjectKind::File);
        let child2 = make_test_meta("file_2", Some("folder_1"), "b.txt", ObjectKind::File);
        let other = make_test_meta("file_3", None, "root_file.txt", ObjectKind::File);

        store
            .batch_upsert_objects(&[folder.clone(), child1, child2, other])
            .expect("batch upsert");

        let folder_children = store
            .list_children(Some(&folder.id))
            .expect("list folder children");
        assert_eq!(folder_children.len(), 2);
        assert_eq!(folder_children[0].name, "a.txt");
        assert_eq!(folder_children[1].name, "b.txt");

        let root_children = store.list_children(None).expect("list root children");
        assert_eq!(root_children.len(), 2);
    }

    #[test]
    fn lookup_by_name_success_and_miss() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let file = make_test_meta("file_1", None, "report.pdf", ObjectKind::File);
        store.upsert_object(&file).expect("upsert");

        let found = store.lookup_by_name(None, "report.pdf").expect("lookup");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, file.id);

        let missing = store.lookup_by_name(None, "missing.pdf").expect("lookup");
        assert!(missing.is_none());
    }

    #[test]
    fn disambiguates_duplicate_remote_names() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let file1 = make_test_meta("id_1", None, "document.pdf", ObjectKind::File);
        let file2 = make_test_meta("id_2", None, "document.pdf", ObjectKind::File);
        let file3 = make_test_meta("id_3", None, "document.pdf", ObjectKind::File);

        store.upsert_object(&file1).expect("upsert 1");
        store.upsert_object(&file2).expect("upsert 2");
        store.upsert_object(&file3).expect("upsert 3");

        let obj1 = store.get_object(&file1.id).expect("get").unwrap();
        let obj2 = store.get_object(&file2.id).expect("get").unwrap();
        let obj3 = store.get_object(&file3.id).expect("get").unwrap();

        assert_eq!(obj1.name, "document.pdf");
        assert_eq!(obj1.remote_name, "document.pdf");

        assert_eq!(obj2.name, "document (1).pdf");
        assert_eq!(obj2.remote_name, "document.pdf");

        assert_eq!(obj3.name, "document (2).pdf");
        assert_eq!(obj3.remote_name, "document.pdf");
    }

    #[test]
    fn deletion_and_tombstones() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let file = make_test_meta("del_1", None, "delete_me.txt", ObjectKind::File);
        store.upsert_object(&file).expect("upsert");

        assert!(store.get_object(&file.id).expect("get").is_some());
        store.mark_deleted(&file.id).expect("mark deleted");

        assert!(store.get_object(&file.id).expect("get").is_none());
        assert!(store
            .lookup_by_name(None, "delete_me.txt")
            .expect("lookup")
            .is_none());

        let purged = store.purge_tombstones().expect("purge");
        assert_eq!(purged, 1);
    }

    #[test]
    fn batch_apply_changes_advances_checkpoint_atomically() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let account = AccountId("acc_test_123".into());

        let changes = vec![
            Change::Upsert(make_test_meta("f1", None, "file1.txt", ObjectKind::File)),
            Change::Upsert(make_test_meta("f2", None, "file2.txt", ObjectKind::File)),
        ];

        store
            .batch_apply_changes(&changes, Some("token_page_1"), &account)
            .expect("batch apply");

        assert!(store
            .get_object(&FileId("f1".into()))
            .expect("get")
            .is_some());
        assert!(store
            .get_object(&FileId("f2".into()))
            .expect("get")
            .is_some());

        let checkpoint = store.get_checkpoint(&account).expect("checkpoint");
        assert_eq!(checkpoint.as_deref(), Some("token_page_1"));

        let delete_changes = vec![Change::Delete {
            id: FileId("f1".into()),
        }];
        store
            .batch_apply_changes(&delete_changes, Some("token_page_2"), &account)
            .expect("apply delete");

        assert!(store
            .get_object(&FileId("f1".into()))
            .expect("get")
            .is_none());
        let checkpoint2 = store.get_checkpoint(&account).expect("checkpoint");
        assert_eq!(checkpoint2.as_deref(), Some("token_page_2"));
    }

    #[test]
    fn rename_object_updates_name_and_disambiguates() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let file1 = make_test_meta("f1", None, "file1.txt", ObjectKind::File);
        let file2 = make_test_meta("f2", None, "file2.txt", ObjectKind::File);
        store.upsert_object(&file1).expect("upsert 1");
        store.upsert_object(&file2).expect("upsert 2");

        let renamed = store
            .rename_object(&file1.id, "file2.txt")
            .expect("rename with collision");
        assert_eq!(renamed.name, "file2 (1).txt");
        assert_eq!(renamed.remote_name, "file2.txt");

        let normal_rename = store
            .rename_object(&file1.id, "unique.txt")
            .expect("rename unique");
        assert_eq!(normal_rename.name, "unique.txt");
        assert_eq!(normal_rename.remote_name, "unique.txt");
    }

    #[test]
    fn move_object_updates_parent_and_name() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let folder = make_test_meta("folder_1", None, "Folder", ObjectKind::Directory);
        let file = make_test_meta("f1", None, "file.txt", ObjectKind::File);
        store.upsert_object(&folder).expect("upsert folder");
        store.upsert_object(&file).expect("upsert file");

        let moved = store
            .move_object(&file.id, Some(&folder.id), "file_moved.txt")
            .expect("move");
        assert_eq!(moved.parent_id, Some(folder.id.clone()));
        assert_eq!(moved.name, "file_moved.txt");

        let children = store
            .list_children(Some(&folder.id))
            .expect("list children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, file.id);
    }

    #[test]
    fn update_size_and_mtime_and_trash() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let file = make_test_meta("f1", None, "data.bin", ObjectKind::File);
        store.upsert_object(&file).expect("upsert");

        store
            .update_size_and_mtime(&file.id, 4096, Some("2026-09-29T10:00:00Z"))
            .expect("update size");
        let obj = store.get_object(&file.id).expect("get").unwrap();
        assert_eq!(obj.size_bytes, Some(4096));
        assert_eq!(obj.modified_at.as_deref(), Some("2026-09-29T10:00:00Z"));

        store.mark_trashed(&file.id).expect("trash");
        assert!(store.get_object(&file.id).expect("get").is_none());
    }

    #[test]
    fn has_children_detects_active_entries() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let folder = make_test_meta("dir1", None, "Dir", ObjectKind::Directory);
        let child = make_test_meta("c1", Some("dir1"), "c.txt", ObjectKind::File);
        store.upsert_object(&folder).expect("upsert folder");
        assert!(!store.has_children(&folder.id).expect("empty check"));

        store.upsert_object(&child).expect("upsert child");
        assert!(store.has_children(&folder.id).expect("non-empty check"));

        store.mark_deleted(&child.id).expect("delete child");
        assert!(!store
            .has_children(&folder.id)
            .expect("empty after delete check"));
    }

    #[test]
    fn lookup_by_name_is_case_insensitive() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let file = make_test_meta("f_case", None, "MyReport.PDF", ObjectKind::File);
        store.upsert_object(&file).expect("upsert");

        let found_lower = store.lookup_by_name(None, "myreport.pdf").expect("lower");
        assert!(found_lower.is_some());
        assert_eq!(found_lower.unwrap().id.0, "f_case");

        let found_upper = store.lookup_by_name(None, "MYREPORT.PDF").expect("upper");
        assert!(found_upper.is_some());
        assert_eq!(found_upper.unwrap().id.0, "f_case");
    }

    #[test]
    fn slash_in_remote_name_is_sanitized_locally() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let file = make_test_meta("f_slash", None, "report/2026\\final.pdf", ObjectKind::File);
        store.upsert_object(&file).expect("upsert");

        let obj = store.get_object(&file.id).expect("get").unwrap();
        assert_eq!(obj.name, "report_2026_final.pdf");
        assert_eq!(obj.remote_name, "report/2026\\final.pdf");

        let found = store
            .lookup_by_name(None, "report_2026_final.pdf")
            .expect("lookup");
        assert!(found.is_some());
    }

    #[test]
    fn is_descendant_of_detects_ancestor_chains() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let d1 = make_test_meta("d1", None, "D1", ObjectKind::Directory);
        let d2 = make_test_meta("d2", Some("d1"), "D2", ObjectKind::Directory);
        let d3 = make_test_meta("d3", Some("d2"), "D3", ObjectKind::Directory);
        let other = make_test_meta("other", None, "Other", ObjectKind::Directory);

        store
            .batch_upsert_objects(&[d1.clone(), d2.clone(), d3.clone(), other.clone()])
            .expect("upsert");

        assert!(store
            .is_descendant_of(&d3.id, &d1.id)
            .expect("check d3 in d1"));
        assert!(store
            .is_descendant_of(&d2.id, &d1.id)
            .expect("check d2 in d1"));
        assert!(store.is_descendant_of(&d1.id, &d1.id).expect("self check"));
        assert!(!store
            .is_descendant_of(&d1.id, &d3.id)
            .expect("reverse check"));
        assert!(!store
            .is_descendant_of(&other.id, &d1.id)
            .expect("unrelated check"));
    }

    #[test]
    fn tracks_synced_directories_and_version_update() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let dir_id = FileId("folder_1".into());

        assert!(!store.is_directory_synced(Some(&dir_id)).expect("check"));
        assert!(!store.is_directory_synced(None).expect("check root"));

        store
            .mark_directory_synced(Some(&dir_id))
            .expect("mark dir");
        store.mark_directory_synced(None).expect("mark root");

        assert!(store.is_directory_synced(Some(&dir_id)).expect("check"));
        assert!(store.is_directory_synced(None).expect("check root"));

        let file = make_test_meta("f1", None, "file.bin", ObjectKind::File);
        store.upsert_object(&file).expect("upsert");

        store
            .update_size_mtime_version(&file.id, 2048, Some("2026-09-30T10:00:00Z"), Some("v2"))
            .expect("update");
        let retrieved = store.get_object(&file.id).expect("get").unwrap();
        assert_eq!(retrieved.size_bytes, Some(2048));
        assert_eq!(retrieved.version.as_deref(), Some("v2"));
    }

    #[test]
    fn test_staging_journal_lifecycle() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let file_id = FileId("staging_file_1".into());
        let parent_id = FileId("parent_1".into());

        assert!(store.list_uncommitted_staging().unwrap().is_empty());

        store
            .record_staging_entry(1001, &file_id, "C:\\staging\\1001.tmp", Some(&parent_id))
            .expect("record staging");

        let entries = store.list_uncommitted_staging().expect("list staging");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].handle_id, 1001);
        assert_eq!(entries[0].file_id, file_id);
        assert_eq!(entries[0].state, StagingState::Staging);
        assert_eq!(entries[0].session_uri, None);
        assert_eq!(entries[0].uploaded_bytes, 0);

        store
            .update_staging_session(1001, "https://upload.example.com/resumable/123", 5242880)
            .expect("update session");
        let entries_session = store.list_uncommitted_staging().expect("list staging");
        assert_eq!(
            entries_session[0].session_uri.as_deref(),
            Some("https://upload.example.com/resumable/123")
        );
        assert_eq!(entries_session[0].uploaded_bytes, 5242880);

        store
            .update_staging_state(1001, StagingState::Uploading)
            .expect("update state");
        let entries2 = store.list_uncommitted_staging().expect("list staging");
        assert_eq!(entries2[0].state, StagingState::Uploading);

        store.remove_staging_entry(1001).expect("remove entry");
        assert!(store.list_uncommitted_staging().unwrap().is_empty());
    }

    #[test]
    fn concurrent_reader_during_writer_transaction() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("metadata.db");
        let store = std::sync::Arc::new(MetadataStore::open(&db_path).expect("open store"));

        let initial_file = make_test_meta("initial_1", None, "init.txt", ObjectKind::File);
        store.upsert_object(&initial_file).expect("upsert initial");

        let (tx_start, rx_start) = std::sync::mpsc::channel();
        let (tx_done, rx_done) = std::sync::mpsc::channel();

        let store_writer = store.clone();
        let writer_handle = std::thread::spawn(move || {
            tx_start.send(()).unwrap();
            let objects: Vec<_> = (0..50)
                .map(|i| {
                    make_test_meta(
                        &format!("batch_{i}"),
                        None,
                        &format!("f_{i}.txt"),
                        ObjectKind::File,
                    )
                })
                .collect();
            store_writer
                .batch_upsert_objects(&objects)
                .expect("batch upsert");
            tx_done.send(()).unwrap();
        });

        rx_start.recv().unwrap();

        let obj = store.get_object(&initial_file.id).expect("reader query");
        assert!(obj.is_some());
        assert_eq!(obj.unwrap().name, "init.txt");

        writer_handle.join().unwrap();
        rx_done.recv().unwrap();

        let all_root = store.list_children(None).expect("list children");
        assert_eq!(all_root.len(), 51);
    }

    #[test]
    fn replace_file_id_and_sync_queue() {
        let store = MetadataStore::open_in_memory().expect("open store");
        let local_id = FileId::new_local();
        let local_obj = StoredObject {
            id: local_id.clone(),
            parent_id: None,
            name: "test.txt".into(),
            remote_name: "test.txt".into(),
            kind: ObjectKind::File,
            size_bytes: Some(1024),
            mime_type: Some("text/plain".into()),
            created_at: None,
            modified_at: None,
            version: None,
            sync_status: SyncStatus::Pending,
            deleted: false,
        };
        store.insert_new_object(&local_obj).expect("insert local");
        store
            .record_staging_entry(2001, &local_id, "C:\\staging\\2001.tmp", None)
            .expect("record staging");

        let (count, bytes) = store.get_pending_sync_stats().expect("pending stats");
        assert_eq!(count, 1);
        assert_eq!(bytes, 1024);

        let remote_obj = StoredObject {
            id: FileId("remote_123".into()),
            parent_id: None,
            name: "test.txt".into(),
            remote_name: "test.txt".into(),
            kind: ObjectKind::File,
            size_bytes: Some(1024),
            mime_type: Some("text/plain".into()),
            created_at: Some("2026-10-01T00:00:00Z".into()),
            modified_at: Some("2026-10-01T00:00:00Z".into()),
            version: Some("v1".into()),
            sync_status: SyncStatus::Synced,
            deleted: false,
        };
        store
            .replace_file_id(&local_id, &remote_obj)
            .expect("replace id");

        let old_lookup = store.get_object(&local_id).expect("lookup old");
        assert!(old_lookup.is_none());

        let new_lookup = store
            .get_object(&FileId("remote_123".into()))
            .expect("lookup new");
        assert!(new_lookup.is_some());
        assert_eq!(new_lookup.unwrap().id.0, "remote_123");

        store
            .enqueue_delete(&FileId("remote_123".into()), None)
            .expect("enqueue delete");
        let uncommitted = store.list_uncommitted_staging().expect("list uncommitted");
        assert_eq!(uncommitted.len(), 2);
        assert_eq!(uncommitted[1].direction, "delete");
    }
}

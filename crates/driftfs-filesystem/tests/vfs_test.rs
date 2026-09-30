use std::sync::Arc;
use std::time::Duration;

use driftfs_cache::{BoundedChunkCache, ChunkKey, DEFAULT_CHUNK_SIZE};
use driftfs_core::FileId;
use driftfs_filesystem::{DriftFsVfs, OpenFlags, VfsError, VfsNodeType, ROOT_INODE};
use driftfs_metadata::MetadataStore;
use driftfs_provider::ObjectKind;
use driftfs_testkit::{create_metadata, MockProvider};

fn setup_vfs() -> (DriftFsVfs<MockProvider>, tempfile::TempDir) {
    let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
    let provider = MockProvider::new();

    let docs_folder = create_metadata(
        "folder_docs",
        "Documents",
        None,
        ObjectKind::Directory,
        None,
        Some("application/vnd.google-apps.folder"),
    );
    let notes_folder = create_metadata(
        "folder_notes",
        "Notes",
        Some("folder_docs"),
        ObjectKind::Directory,
        None,
        Some("application/vnd.google-apps.folder"),
    );
    let text_file = create_metadata(
        "file_notes",
        "meeting.txt",
        Some("folder_notes"),
        ObjectKind::File,
        Some(1024),
        Some("text/plain"),
    );
    let root_file = create_metadata(
        "file_root",
        "readme.txt",
        None,
        ObjectKind::File,
        Some(512),
        Some("text/plain"),
    );
    let gdoc_file = create_metadata(
        "gdoc_plan",
        "Quarterly Roadmap",
        None,
        ObjectKind::File,
        None,
        Some("application/vnd.google-apps.document"),
    );

    store
        .batch_upsert_objects(&[docs_folder, notes_folder, text_file, root_file, gdoc_file])
        .expect("batch upsert");

    provider.add_directory("folder_docs", "Documents", None);
    provider.add_directory("folder_notes", "Notes", Some("folder_docs"));
    provider.add_file("file_notes", "meeting.txt", Some("folder_notes"), 1024);
    provider.add_file("file_root", "readme.txt", None, 512);
    provider.add_directory("root", "My Drive", None);

    let staging_dir = tempfile::tempdir().expect("staging dir");
    let vfs = DriftFsVfs::new(store, Arc::new(provider), staging_dir.path().to_path_buf())
        .expect("create vfs")
        .with_root_folder_id(driftfs_core::FileId("root".into()));

    (vfs, staging_dir)
}

#[tokio::test]
async fn root_attributes_and_directory_enumeration() {
    let (vfs, _dir) = setup_vfs();

    let root_attr = vfs.getattr("/").expect("root attr");
    assert_eq!(root_attr.ino, ROOT_INODE);
    assert_eq!(root_attr.kind, VfsNodeType::Directory);

    let root_handle = vfs.opendir("/").expect("opendir root");
    let entries = vfs.readdir(root_handle).await.expect("readdir root");
    vfs.close(root_handle).expect("close root");

    assert_eq!(entries.len(), 3);
    assert!(entries
        .iter()
        .any(|e| e.name == "Documents" && e.kind == VfsNodeType::Directory));
    assert!(entries
        .iter()
        .any(|e| e.name == "readme.txt" && e.kind == VfsNodeType::File));
    assert!(entries
        .iter()
        .any(|e| e.name == "Quarterly Roadmap.url" && e.kind == VfsNodeType::GoogleDocShortcut));
}

#[tokio::test]
async fn nested_path_lookup_and_file_attributes() {
    let (vfs, _dir) = setup_vfs();

    let attr = vfs
        .getattr("/Documents/Notes/meeting.txt")
        .expect("getattr nested");
    assert_eq!(attr.kind, VfsNodeType::File);
    assert_eq!(attr.size, 1024);

    let not_found = vfs.getattr("/Documents/NonExistent.txt");
    assert_eq!(not_found.unwrap_err(), VfsError::NotFound);
}

#[tokio::test]
async fn google_workspace_url_shortcut_synthesis_and_read() {
    let (vfs, _dir) = setup_vfs();

    let attr = vfs.getattr("Quarterly Roadmap.url").expect("getattr gdoc");
    assert_eq!(attr.kind, VfsNodeType::GoogleDocShortcut);
    assert!(attr.size > 0);

    let handle = vfs
        .open("Quarterly Roadmap.url", OpenFlags::read_only())
        .await
        .expect("open gdoc");
    let content_bytes = vfs.read(handle, 0, 500).await.expect("read gdoc");
    vfs.close(handle).expect("close gdoc");

    let content = String::from_utf8(content_bytes).expect("utf8");
    assert!(content.contains("[InternetShortcut]"));
    assert!(content.contains("https://docs.google.com/document/d/gdoc_plan/edit"));
}

#[tokio::test]
async fn on_demand_range_reads_and_eof() {
    let (vfs, _dir) = setup_vfs();

    let handle = vfs
        .open("readme.txt", OpenFlags::read_only())
        .await
        .expect("open readme");

    let slice1 = vfs.read(handle, 0, 100).await.expect("read 100");
    assert_eq!(slice1.len(), 100);

    let slice2 = vfs.read(handle, 500, 50).await.expect("read near eof");
    assert_eq!(slice2.len(), 12);

    let slice3 = vfs.read(handle, 512, 50).await.expect("read at eof");
    assert!(slice3.is_empty());

    vfs.close(handle).expect("close");
}

#[tokio::test]
async fn write_to_workspace_shortcut_is_denied() {
    let (vfs, _dir) = setup_vfs();

    let write_flags = OpenFlags {
        read: true,
        write: true,
        truncate: false,
        create: false,
        append: false,
    };

    let err = vfs
        .open("Quarterly Roadmap.url", write_flags)
        .await
        .unwrap_err();
    assert_eq!(err, VfsError::AccessDenied);
}

#[tokio::test]
async fn handle_table_lifecycle_and_invalidation() {
    let (vfs, _dir) = setup_vfs();

    let handle = vfs
        .open("readme.txt", OpenFlags::read_only())
        .await
        .expect("open");
    assert_eq!(vfs.handles().count(), 1);

    vfs.close(handle).expect("close");
    assert_eq!(vfs.handles().count(), 0);

    let err = vfs.read(handle, 0, 10).await.unwrap_err();
    assert_eq!(err, VfsError::InvalidHandle);
}

#[tokio::test]
async fn create_file_and_write_then_read_back() {
    let (vfs, _dir) = setup_vfs();

    let (handle, attr) = vfs
        .create_file("/Documents", "new_file.txt")
        .await
        .expect("create");
    assert_eq!(attr.kind, VfsNodeType::File);
    assert_eq!(attr.size, 0);

    let written = vfs.write(handle, 0, b"hello driftfs").expect("write");
    assert_eq!(written, 13);

    let data = vfs.read(handle, 0, 100).await.expect("read");
    assert_eq!(&data, b"hello driftfs");

    vfs.close_async(handle).await.expect("close");
}

#[tokio::test]
async fn mkdir_creates_directory() {
    let (vfs, _dir) = setup_vfs();

    let attr = vfs.mkdir("/Documents/NewSubdir").await.expect("mkdir");
    assert_eq!(attr.kind, VfsNodeType::Directory);

    let get_attr = vfs.getattr("/Documents/NewSubdir").expect("getattr");
    assert_eq!(get_attr.kind, VfsNodeType::Directory);
}

#[tokio::test]
async fn unlink_removes_file() {
    let (vfs, _dir) = setup_vfs();

    vfs.unlink("/readme.txt").await.expect("unlink");

    let err = vfs.getattr("/readme.txt").unwrap_err();
    assert_eq!(err, VfsError::NotFound);
}

#[tokio::test]
async fn rmdir_rejects_nonempty_directory() {
    let (vfs, _dir) = setup_vfs();

    let err = vfs.rmdir("/Documents").await.unwrap_err();
    assert_eq!(err, VfsError::DirectoryNotEmpty);
}

#[tokio::test]
async fn rmdir_empty_directory() {
    let (vfs, _dir) = setup_vfs();

    let _ = vfs.mkdir("/EmptyDir").await.expect("mkdir");
    vfs.rmdir("/EmptyDir").await.expect("rmdir");

    let err = vfs.getattr("/EmptyDir").unwrap_err();
    assert_eq!(err, VfsError::NotFound);
}

#[tokio::test]
async fn rename_file_in_same_directory() {
    let (vfs, _dir) = setup_vfs();

    vfs.rename("/readme.txt", "/readme_renamed.txt", false)
        .await
        .expect("rename");

    let err = vfs.getattr("/readme.txt").unwrap_err();
    assert_eq!(err, VfsError::NotFound);

    let attr = vfs.getattr("/readme_renamed.txt").expect("getattr renamed");
    assert_eq!(attr.kind, VfsNodeType::File);
}

#[tokio::test]
async fn write_at_offset_extends_file() {
    let (vfs, _dir) = setup_vfs();

    let (handle, _) = vfs
        .create_file("/Documents", "offset_test.txt")
        .await
        .expect("create");

    vfs.write(handle, 0, b"AAAA").expect("write1");
    vfs.write(handle, 10, b"BB").expect("write2");

    let entry = vfs.handles().get(handle).expect("get entry");
    assert_eq!(entry.size, 12);

    vfs.close_async(handle).await.expect("close");
}

#[tokio::test]
async fn set_length_truncates_file() {
    let (vfs, _dir) = setup_vfs();

    let (handle, _) = vfs
        .create_file("/Documents", "trunc_test.txt")
        .await
        .expect("create");

    vfs.write(handle, 0, b"abcdefghij").expect("write");
    assert_eq!(vfs.handles().get(handle).unwrap().size, 10);

    vfs.set_length(handle, 5).expect("truncate");
    assert_eq!(vfs.handles().get(handle).unwrap().size, 5);

    let data = vfs.read(handle, 0, 100).await.expect("read");
    assert_eq!(&data, b"abcde");

    vfs.close_async(handle).await.expect("close");
}

#[tokio::test]
async fn open_existing_file_with_write_flag() {
    let (vfs, _dir) = setup_vfs();

    let write_flags = OpenFlags {
        read: true,
        write: true,
        truncate: true,
        create: false,
        append: false,
    };

    let handle = vfs
        .open("readme.txt", write_flags)
        .await
        .expect("open write");
    let entry = vfs.handles().get(handle).unwrap();
    assert!(entry.is_dirty);
    assert_eq!(entry.size, 0);

    vfs.close_async(handle).await.expect("close");
}

#[tokio::test]
async fn move_file_from_subdirectory_to_root() {
    let (vfs, _dir) = setup_vfs();

    let (handle, _) = vfs
        .create_file("/Documents", "sub_file.txt")
        .await
        .expect("create file in subdir");
    vfs.close_async(handle).await.expect("close");

    assert!(vfs
        .resolve_path("/Documents/sub_file.txt")
        .unwrap()
        .is_some());

    vfs.rename("/Documents/sub_file.txt", "/sub_file_root.txt", false)
        .await
        .expect("move to root");
    assert_eq!(
        vfs.resolve_path("/Documents/sub_file.txt").unwrap_err(),
        VfsError::NotFound
    );
    let moved_obj = vfs
        .resolve_path("/sub_file_root.txt")
        .unwrap()
        .expect("found at root");
    assert_eq!(moved_obj.name, "sub_file_root.txt");
    assert_eq!(moved_obj.parent_id, None);
}

#[tokio::test]
async fn dirty_close_preserves_staging_file() {
    let (vfs, _dir) = setup_vfs();

    let write_flags = OpenFlags {
        read: true,
        write: true,
        truncate: false,
        create: false,
        append: false,
    };

    let handle = vfs
        .open("readme.txt", write_flags)
        .await
        .expect("open write");
    vfs.write(handle, 0, b"dirty uncommitted data")
        .expect("write");

    let entry = vfs.handles().get(handle).unwrap();
    let staging_path = entry.staging_path.clone().expect("staging path");
    assert!(staging_path.exists());

    vfs.close(handle).expect("close");
    assert!(
        staging_path.exists(),
        "staging file must be preserved on dirty close"
    );
    let _ = std::fs::remove_file(&staging_path);
}

#[tokio::test]
async fn modify_existing_file_preserves_unmodified_content() {
    let (vfs, _dir) = setup_vfs();

    let write_flags = OpenFlags {
        read: true,
        write: true,
        truncate: false,
        create: false,
        append: false,
    };

    let handle = vfs
        .open("readme.txt", write_flags)
        .await
        .expect("open write without truncate");

    vfs.write(handle, 0, b"MODIF").expect("write at offset 0");

    let data = vfs.read(handle, 0, 15).await.expect("read back");
    assert_eq!(&data[..5], b"MODIF");
    assert_eq!(data.len(), 15);

    vfs.close_async(handle).await.expect("close");
}

#[tokio::test]
async fn rename_rejects_cycle_into_descendant() {
    let (vfs, _dir) = setup_vfs();

    // /Documents is parent of /Documents/Notes. Moving /Documents to /Documents/Notes/Sub is invalid.
    let err = vfs
        .rename("/Documents", "/Documents/Notes/Sub", false)
        .await
        .unwrap_err();
    assert_eq!(err, VfsError::InvalidPath);
}

#[tokio::test]
async fn rename_collision_handling() {
    let (vfs, _dir) = setup_vfs();

    let (h, _) = vfs
        .create_file("/Documents", "source.txt")
        .await
        .expect("create");
    vfs.close_async(h).await.expect("close");

    let err = vfs
        .rename("/Documents/source.txt", "/readme.txt", false)
        .await
        .unwrap_err();
    assert_eq!(err, VfsError::AlreadyExists);

    vfs.rename("/Documents/source.txt", "/readme.txt", true)
        .await
        .expect("rename with overwrite");

    assert_eq!(
        vfs.resolve_path("/Documents/source.txt").unwrap_err(),
        VfsError::NotFound
    );
    assert!(vfs.resolve_path("/readme.txt").unwrap().is_some());
}

#[tokio::test]
async fn cached_read_populates_cache_and_serves_data() {
    let (vfs, _staging) = setup_vfs();
    let cache_dir = tempfile::tempdir().expect("cache dir");
    let cache = Arc::new(
        BoundedChunkCache::new(
            cache_dir.path().to_path_buf(),
            10 * 1024 * 1024,
            DEFAULT_CHUNK_SIZE,
        )
        .expect("cache open"),
    );
    let vfs = vfs.with_chunk_cache(Arc::clone(&cache));

    let handle = vfs
        .open("/readme.txt", OpenFlags::read_only())
        .await
        .expect("open readme");

    let data1 = vfs.read(handle, 0, 512).await.expect("read 1");
    assert_eq!(data1.len(), 512);
    assert_eq!(vfs.provider().read_count(), 1);

    let key = ChunkKey::new(FileId("file_root".into()), "1", 0);
    assert!(cache.get(&key).await.unwrap().is_some());

    let data2 = vfs.read(handle, 0, 512).await.expect("read 2");
    assert_eq!(data2, data1);
    assert_eq!(vfs.provider().read_count(), 1);

    vfs.close(handle).expect("close");
}

#[tokio::test]
async fn multi_chunk_spanning_read() {
    let (vfs, _staging) = setup_vfs();
    let cache_dir = tempfile::tempdir().expect("cache dir");
    let cache = Arc::new(
        BoundedChunkCache::new(
            cache_dir.path().to_path_buf(),
            10 * 1024 * 1024,
            DEFAULT_CHUNK_SIZE,
        )
        .expect("cache open"),
    );
    let vfs = vfs.with_chunk_cache(Arc::clone(&cache));

    let total_size = DEFAULT_CHUNK_SIZE * 2 + 1024;
    let full_content: Vec<u8> = (0..total_size).map(|i| (i % 251) as u8).collect();

    let file_meta = create_metadata(
        "file_multi",
        "multichunk.bin",
        None,
        ObjectKind::File,
        Some(total_size as u64),
        Some("application/octet-stream"),
    );
    vfs.metadata()
        .batch_upsert_objects(&[file_meta])
        .expect("upsert multi");
    vfs.provider()
        .add_file_with_bytes("file_multi", "multichunk.bin", None, &full_content);

    let handle = vfs
        .open("/multichunk.bin", OpenFlags::read_only())
        .await
        .expect("open multichunk");

    let offset = 400 * 1024usize;
    let length = 300 * 1024usize;
    let read_slice = vfs
        .read(handle, offset as u64, length)
        .await
        .expect("spanning read");
    assert_eq!(read_slice.len(), length);
    assert_eq!(read_slice, full_content[offset..offset + length]);

    let key0 = ChunkKey::new(FileId("file_multi".into()), "1", 0);
    let key1 = ChunkKey::new(FileId("file_multi".into()), "1", 1);
    assert!(cache.get(&key0).await.unwrap().is_some());
    assert!(cache.get(&key1).await.unwrap().is_some());

    vfs.close(handle).expect("close");
}

#[tokio::test]
async fn sequential_read_triggers_prefetch() {
    let (vfs, _staging) = setup_vfs();
    let cache_dir = tempfile::tempdir().expect("cache dir");
    let cache = Arc::new(
        BoundedChunkCache::new(
            cache_dir.path().to_path_buf(),
            10 * 1024 * 1024,
            DEFAULT_CHUNK_SIZE,
        )
        .expect("cache open"),
    );
    let vfs = vfs.with_chunk_cache(Arc::clone(&cache)).with_prefetch(true);

    let total_size = DEFAULT_CHUNK_SIZE * 3;
    let full_content: Vec<u8> = (0..total_size).map(|i| (i % 241) as u8).collect();

    let file_meta = create_metadata(
        "file_stream",
        "stream.bin",
        None,
        ObjectKind::File,
        Some(total_size as u64),
        Some("application/octet-stream"),
    );
    vfs.metadata()
        .batch_upsert_objects(&[file_meta])
        .expect("upsert stream");
    vfs.provider()
        .add_file_with_bytes("file_stream", "stream.bin", None, &full_content);

    let handle = vfs
        .open("/stream.bin", OpenFlags::read_only())
        .await
        .expect("open stream");

    let _ = vfs.read(handle, 0, 64 * 1024).await.expect("read 0");
    let _ = vfs
        .read(handle, 64 * 1024, 64 * 1024)
        .await
        .expect("read 1");
    let _ = vfs
        .read(handle, 128 * 1024, 64 * 1024)
        .await
        .expect("read 2");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let key1 = ChunkKey::new(FileId("file_stream".into()), "1", 1);
    assert!(
        cache.get(&key1).await.unwrap().is_some(),
        "Chunk 1 should have been prefetched into cache"
    );

    vfs.close(handle).expect("close");
}

#[tokio::test]
async fn dirty_close_invalidates_chunk_cache() {
    let (vfs, _staging) = setup_vfs();
    let cache_dir = tempfile::tempdir().expect("cache dir");
    let cache = Arc::new(
        BoundedChunkCache::new(
            cache_dir.path().to_path_buf(),
            10 * 1024 * 1024,
            DEFAULT_CHUNK_SIZE,
        )
        .expect("cache open"),
    );
    let vfs = vfs.with_chunk_cache(Arc::clone(&cache));

    let r_handle = vfs
        .open("/readme.txt", OpenFlags::read_only())
        .await
        .expect("open read");
    let _ = vfs.read(r_handle, 0, 512).await.expect("read");
    vfs.close(r_handle).expect("close read");

    let key = ChunkKey::new(FileId("file_root".into()), "1", 0);
    assert!(cache.get(&key).await.unwrap().is_some());

    let w_handle = vfs
        .open("/readme.txt", OpenFlags::write_only())
        .await
        .expect("open write");
    vfs.write(w_handle, 0, b"mutated data").expect("write");
    vfs.close_async(w_handle).await.expect("close write");

    assert!(cache.get(&key).await.unwrap().is_none());
}

#[tokio::test]
async fn unlink_invalidates_chunk_cache() {
    let (vfs, _staging) = setup_vfs();
    let cache_dir = tempfile::tempdir().expect("cache dir");
    let cache = Arc::new(
        BoundedChunkCache::new(
            cache_dir.path().to_path_buf(),
            10 * 1024 * 1024,
            DEFAULT_CHUNK_SIZE,
        )
        .expect("cache open"),
    );
    let vfs = vfs.with_chunk_cache(Arc::clone(&cache));

    let r_handle = vfs
        .open("/readme.txt", OpenFlags::read_only())
        .await
        .expect("open read");
    let _ = vfs.read(r_handle, 0, 512).await.expect("read");
    vfs.close(r_handle).expect("close read");

    let key = ChunkKey::new(FileId("file_root".into()), "1", 0);
    assert!(cache.get(&key).await.unwrap().is_some());

    vfs.unlink("/readme.txt").await.expect("unlink");

    assert!(cache.get(&key).await.unwrap().is_none());
}

#[tokio::test]
async fn crash_recovery_uploads_pending_staging_and_cleans_orphans() {
    let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
    let provider = Arc::new(MockProvider::new());
    provider.add_directory("root", "My Drive", None);
    provider.add_file("file1", "pending.txt", None, 0);

    let file_meta = create_metadata(
        "file1",
        "pending.txt",
        None,
        ObjectKind::File,
        Some(0),
        Some("text/plain"),
    );
    store.upsert_object(&file_meta).expect("upsert");

    let staging_dir = tempfile::tempdir().expect("staging dir");

    let staged_file_path = staging_dir.path().join("999.tmp");
    std::fs::write(&staged_file_path, b"data persisted before crash").expect("write staged file");
    store
        .record_staging_entry(
            999,
            &FileId("file1".into()),
            &staged_file_path.to_string_lossy(),
            None,
        )
        .expect("record staging");

    let orphan_file_path = staging_dir.path().join("orphan.tmp");
    std::fs::write(&orphan_file_path, b"garbage data").expect("write orphan");

    let vfs = DriftFsVfs::new(
        Arc::clone(&store),
        Arc::clone(&provider),
        staging_dir.path().to_path_buf(),
    )
    .expect("create vfs");

    assert!(orphan_file_path.exists());
    assert!(staged_file_path.exists());

    let recovered = vfs.recover_pending_uploads().await.expect("recover");
    assert_eq!(recovered, 1);

    assert!(!staged_file_path.exists());
    let pending = store.list_uncommitted_staging().expect("list uncommitted");
    assert!(pending.is_empty());

    assert!(!orphan_file_path.exists());

    let obj = store
        .get_object(&FileId("file1".into()))
        .expect("get obj")
        .unwrap();
    assert_eq!(obj.size_bytes, Some(27));
}

use std::sync::Arc;

use driftfs_cache::BoundedChunkCache;
use driftfs_core::FileId;
use driftfs_filesystem::{DriftFsVfs, OpenFlags};
use driftfs_metadata::MetadataStore;
use driftfs_provider::ObjectKind;
use driftfs_testkit::{create_metadata, MockProvider};

fn setup_vfs(
    file_count: usize,
    file_size: usize,
) -> (
    DriftFsVfs<MockProvider>,
    Arc<MetadataStore>,
    Arc<MockProvider>,
    tempfile::TempDir,
) {
    let store = Arc::new(MetadataStore::open_in_memory().expect("open store"));
    let provider = Arc::new(MockProvider::new());
    provider.add_directory("root", "My Drive", None);

    let mut metas = Vec::new();
    for i in 0..file_count {
        let id = format!("file_{i}");
        let name = format!("data_{i}.bin");
        let data = vec![(i % 256) as u8; file_size];
        provider.add_file_with_bytes(&id, &name, None, &data);

        metas.push(create_metadata(
            &id,
            &name,
            None,
            ObjectKind::File,
            Some(file_size as u64),
            Some("application/octet-stream"),
        ));
    }
    store.batch_upsert_objects(&metas).expect("batch upsert");

    let staging_dir = tempfile::tempdir().expect("staging dir");
    let vfs = DriftFsVfs::new(
        Arc::clone(&store),
        Arc::clone(&provider),
        staging_dir.path().to_path_buf(),
    )
    .expect("create vfs")
    .with_root_folder_id(FileId("root".into()));

    (vfs, store, provider, staging_dir)
}

#[tokio::test]
async fn concurrent_read_write_torture() {
    let (vfs, _store, _provider, _dir) = setup_vfs(10, 2048);
    let vfs = Arc::new(vfs);

    let mut handles = Vec::new();

    // 8 Readers concurrently reading slices
    for worker_id in 0..8 {
        let vfs_c = Arc::clone(&vfs);
        handles.push(tokio::spawn(async move {
            for iter in 0..25 {
                let file_idx = (worker_id + iter) % 10;
                let path = format!("/data_{file_idx}.bin");
                let handle = vfs_c
                    .open(&path, OpenFlags::read_only())
                    .await
                    .expect("open read");
                let data = vfs_c.read(handle, 0, 512).await.expect("read chunk");
                assert_eq!(data.len(), 512);
                let expected_byte = (file_idx % 256) as u8;
                assert_eq!(data[0], expected_byte);
                vfs_c.close(handle).expect("close");
            }
        }));
    }

    // 4 Writers creating, writing, and reading back temporary files
    for worker_id in 0..4 {
        let vfs_c = Arc::clone(&vfs);
        handles.push(tokio::spawn(async move {
            for iter in 0..15 {
                let file_name = format!("worker_{worker_id}_file_{iter}.tmp");
                let (handle, _attr) = vfs_c
                    .create_file("/", &file_name)
                    .await
                    .expect("create file");

                let payload = format!("payload from worker {worker_id} iteration {iter}");
                vfs_c.write(handle, 0, payload.as_bytes()).expect("write");
                let read_back = vfs_c
                    .read(handle, 0, payload.len())
                    .await
                    .expect("read back");
                assert_eq!(read_back, payload.as_bytes());

                vfs_c.close_async(handle).await.expect("close async");

                let unlink_path = format!("/{file_name}");
                vfs_c.unlink(&unlink_path).await.expect("unlink");
            }
        }));
    }

    for h in handles {
        h.await.expect("worker task failed");
    }
}

#[tokio::test]
async fn cache_pressure_lru_torture() {
    let (vfs, _store, _provider, _dir) = setup_vfs(30, 8192);

    let cache_dir = tempfile::tempdir().expect("cache dir");
    // Limit cache to 32 KB (8 chunks of 4 KB) to force continuous evictions
    let cache = Arc::new(
        BoundedChunkCache::new(cache_dir.path().to_path_buf(), 32 * 1024, 4 * 1024)
            .expect("cache init"),
    );
    let vfs = Arc::new(vfs.with_chunk_cache(cache));

    let mut handles = Vec::new();
    for worker_id in 0..6 {
        let vfs_c = Arc::clone(&vfs);
        handles.push(tokio::spawn(async move {
            for iter in 0..20 {
                let file_idx = (worker_id * 5 + iter) % 30;
                let path = format!("/data_{file_idx}.bin");
                let handle = vfs_c
                    .open(&path, OpenFlags::read_only())
                    .await
                    .expect("open");
                let data = vfs_c.read(handle, 0, 8192).await.expect("read");
                assert_eq!(data.len(), 8192);
                vfs_c.close(handle).expect("close");
            }
        }));
    }

    for h in handles {
        h.await.expect("cache worker failed");
    }
}

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use driftfs_core::FileId;
use tracing::{debug, instrument, warn};

use crate::chunk::{ChunkKey, DEFAULT_CHUNK_SIZE};
use crate::error::{CacheError, Result};
use crate::lru::LruTracker;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct BoundedChunkCache {
    cache_dir: PathBuf,
    chunk_size: usize,
    lru: Mutex<LruTracker>,
}

impl BoundedChunkCache {
    #[instrument(skip_all, fields(cache_dir = %cache_dir.display(), max_size = max_size_bytes))]
    pub fn new(cache_dir: PathBuf, max_size_bytes: u64, chunk_size: usize) -> Result<Self> {
        fs::create_dir_all(&cache_dir)?;

        let mut tracker = LruTracker::new(max_size_bytes);
        if let Ok(entries) = fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("chunk") {
                    if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Some(key) = Self::parse_chunk_filename(file_name) {
                            if let Ok(metadata) = fs::metadata(&path) {
                                tracker.insert(key, metadata.len());
                                continue;
                            }
                        }
                    }
                    let _ = fs::remove_file(&path);
                }
            }
        }

        let evicted = tracker.prepare_eviction(0);
        for key in evicted {
            let path = cache_dir.join(key.to_filename());
            let _ = fs::remove_file(path);
        }

        debug!(
            loaded_bytes = tracker.current_size(),
            max_bytes = tracker.max_size(),
            "bounded chunk cache initialized"
        );

        Ok(Self {
            cache_dir,
            chunk_size: if chunk_size == 0 {
                DEFAULT_CHUNK_SIZE
            } else {
                chunk_size
            },
            lru: Mutex::new(tracker),
        })
    }

    fn lock_lru(&self) -> Result<std::sync::MutexGuard<'_, LruTracker>> {
        self.lru.lock().map_err(|_| CacheError::LockPoisoned)
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    pub fn current_size(&self) -> u64 {
        self.lock_lru().map(|g| g.current_size()).unwrap_or(0)
    }

    pub fn max_size(&self) -> u64 {
        self.lock_lru().map(|g| g.max_size()).unwrap_or(0)
    }

    pub fn reserved_size(&self) -> u64 {
        self.lock_lru().map(|g| g.reserved_size()).unwrap_or(0)
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    #[instrument(skip(self), level = "trace")]
    pub async fn get(&self, key: &ChunkKey) -> Result<Option<Vec<u8>>> {
        let expected_size = {
            let guard = self.lock_lru()?;
            match guard.get_size(key) {
                Some(sz) => sz,
                None => return Ok(None),
            }
        };

        let chunk_path = self.chunk_path(key);
        match tokio::fs::read(&chunk_path).await {
            Ok(bytes) => {
                if bytes.len() as u64 != expected_size {
                    warn!(
                        key = %key,
                        expected = expected_size,
                        actual = bytes.len(),
                        "cache chunk size mismatch; purging corrupted entry"
                    );
                    self.purge_entry(key, &chunk_path).await;
                    return Ok(None);
                }

                if let Ok(mut guard) = self.lock_lru() {
                    guard.touch(key);
                }
                Ok(Some(bytes))
            }
            Err(e) => {
                warn!(
                    key = %key,
                    error = %e,
                    "failed to read cache chunk from disk; purging entry"
                );
                self.purge_entry(key, &chunk_path).await;
                Ok(None)
            }
        }
    }

    #[instrument(skip(self, data), level = "trace")]
    pub async fn put(&self, key: ChunkKey, data: &[u8]) -> Result<()> {
        let data_len = data.len() as u64;
        let evicted = {
            let mut guard = self.lock_lru()?;
            guard.prepare_eviction(data_len)
        };

        for ev_key in evicted {
            let path = self.chunk_path(&ev_key);
            let _ = tokio::fs::remove_file(path).await;
        }

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = self.cache_dir.join(format!(
            "tmp_{}_{}_{}.tmp",
            std::process::id(),
            nanos,
            counter
        ));

        if let Err(e) = tokio::fs::write(&tmp_path, data).await {
            if let Ok(mut guard) = self.lock_lru() {
                guard.cancel_reservation(data_len);
            }
            return Err(CacheError::Io(e));
        }

        let dest_path = self.chunk_path(&key);
        if let Err(e) = tokio::fs::rename(&tmp_path, &dest_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            if let Ok(mut guard) = self.lock_lru() {
                guard.cancel_reservation(data_len);
            }
            return Err(CacheError::Io(e));
        }

        self.lock_lru()?.insert(key, data_len);
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn invalidate_file(&self, file_id: &FileId) -> Result<()> {
        let keys = {
            let mut guard = self.lock_lru()?;
            let keys = guard.keys_for_file(file_id);
            for k in &keys {
                guard.remove(k);
            }
            keys
        };

        for key in keys {
            let path = self.chunk_path(&key);
            let _ = tokio::fs::remove_file(path).await;
        }

        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    pub async fn clear(&self) -> Result<()> {
        let keys = {
            let mut guard = self.lock_lru()?;
            guard.drain_all()
        };

        for key in keys {
            let path = self.chunk_path(&key);
            let _ = tokio::fs::remove_file(path).await;
        }

        Ok(())
    }

    fn chunk_path(&self, key: &ChunkKey) -> PathBuf {
        self.cache_dir.join(key.to_filename())
    }

    async fn purge_entry(&self, key: &ChunkKey, path: &Path) {
        if let Ok(mut guard) = self.lock_lru() {
            guard.remove(key);
        }
        let _ = tokio::fs::remove_file(path).await;
    }

    fn parse_chunk_filename(stem: &str) -> Option<ChunkKey> {
        if stem.contains('@') {
            let parts: Vec<&str> = stem.split('@').collect();
            if parts.len() != 3 {
                return None;
            }
            let file_id = FileId(parts[0].to_string());
            let version = parts[1].to_string();
            let chunk_index: u64 = parts[2].parse().ok()?;
            Some(ChunkKey::new(file_id, version, chunk_index))
        } else {
            let parts: Vec<&str> = stem.rsplitn(3, '_').collect();
            if parts.len() != 3 {
                return None;
            }

            let chunk_index: u64 = parts[0].parse().ok()?;
            let version = parts[1].to_string();
            let file_id = FileId(parts[2].to_string());

            Some(ChunkKey::new(file_id, version, chunk_index))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cache_put_and_get() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = BoundedChunkCache::new(dir.path().to_path_buf(), 1024 * 1024, 512 * 1024)
            .expect("create cache");

        let key = ChunkKey::new(FileId("file1".into()), "v1", 0);
        cache
            .put(key.clone(), b"driftfs chunk payload")
            .await
            .expect("put");

        let retrieved = cache.get(&key).await.expect("get").expect("found");
        assert_eq!(&retrieved, b"driftfs chunk payload");
    }

    #[tokio::test]
    async fn cache_evicts_down_to_low_watermark() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache =
            BoundedChunkCache::new(dir.path().to_path_buf(), 100, 50).expect("create cache");

        let k1 = ChunkKey::new(FileId("f1".into()), "v1", 0);
        let k2 = ChunkKey::new(FileId("f1".into()), "v1", 1);
        let k3 = ChunkKey::new(FileId("f1".into()), "v1", 2);

        cache.put(k1.clone(), &[1u8; 40]).await.expect("put1");
        cache.put(k2.clone(), &[2u8; 40]).await.expect("put2");
        assert_eq!(cache.current_size(), 80);

        // Put 30 bytes: total becomes 110 > 100 max. Eviction down to 85% drops k1 (40 bytes), leaving 40+30=70 <= 85.
        cache.put(k3.clone(), &[3u8; 30]).await.expect("put3");
        assert_eq!(cache.current_size(), 70);

        assert!(cache.get(&k1).await.expect("get1").is_none());
        assert!(cache.get(&k2).await.expect("get2").is_some());
        assert!(cache.get(&k3).await.expect("get3").is_some());
    }

    #[tokio::test]
    async fn cache_corruption_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = BoundedChunkCache::new(dir.path().to_path_buf(), 1024 * 1024, 512 * 1024)
            .expect("create cache");

        let key = ChunkKey::new(FileId("corrupt_file".into()), "v1", 0);
        cache
            .put(key.clone(), b"full valid data")
            .await
            .expect("put");

        let path = cache.chunk_path(&key);
        std::fs::write(&path, b"corrupted").expect("corrupt");

        let res = cache.get(&key).await.expect("get corrupted");
        assert!(res.is_none());
        assert_eq!(cache.current_size(), 0);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn invalidate_file_removes_all_chunks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = BoundedChunkCache::new(dir.path().to_path_buf(), 1024 * 1024, 512 * 1024)
            .expect("create cache");

        let f1 = FileId("target_file".into());
        let f2 = FileId("other_file".into());

        let k1 = ChunkKey::new(f1.clone(), "v1", 0);
        let k2 = ChunkKey::new(f1.clone(), "v1", 1);
        let k3 = ChunkKey::new(f2.clone(), "v1", 0);

        cache.put(k1.clone(), b"f1c0").await.expect("put1");
        cache.put(k2.clone(), b"f1c1").await.expect("put2");
        cache.put(k3.clone(), b"f2c0").await.expect("put3");

        cache.invalidate_file(&f1).await.expect("invalidate");

        assert!(cache.get(&k1).await.unwrap().is_none());
        assert!(cache.get(&k2).await.unwrap().is_none());
        assert!(cache.get(&k3).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn cache_recovery_with_complex_version_and_underscores() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        let key = ChunkKey::new(
            FileId("file_with_underscores_123".into()),
            "2026-09-30T10_00_00_000Z",
            5,
        );

        {
            let cache = BoundedChunkCache::new(path.clone(), 1024 * 1024, 512 * 1024).expect("new");
            cache
                .put(key.clone(), b"persisted chunk")
                .await
                .expect("put");
            assert!(cache.get(&key).await.unwrap().is_some());
        }

        {
            let cache = BoundedChunkCache::new(path, 1024 * 1024, 512 * 1024).expect("reopen");
            let retrieved = cache.get(&key).await.expect("get").expect("found");
            assert_eq!(&retrieved, b"persisted chunk");
        }
    }
}

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Result, VfsError};
use crate::types::VfsHandle;

struct StagingFile {
    path: PathBuf,
    file: std::fs::File,
}

pub struct StagingManager {
    staging_dir: PathBuf,
    files: Mutex<HashMap<u64, StagingFile>>,
}

impl StagingManager {
    pub fn new(staging_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&staging_dir)
            .map_err(|e| VfsError::Io(format!("failed to create staging dir: {e}")))?;

        Ok(Self {
            staging_dir,
            files: Mutex::new(HashMap::new()),
        })
    }

    pub fn cleanup_orphans(&self, preserved_paths: &[PathBuf]) {
        if let Ok(entries) = std::fs::read_dir(&self.staging_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && path.extension().and_then(|s| s.to_str()) == Some("tmp")
                    && !preserved_paths.contains(&path)
                {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }

    pub fn create(&self, handle: VfsHandle) -> Result<PathBuf> {
        let path = self.staging_dir.join(format!("{}.tmp", handle.0));
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| VfsError::Io(format!("failed to create staging file: {e}")))?;

        let staging = StagingFile {
            path: path.clone(),
            file,
        };

        self.files
            .lock()
            .map_err(|_| VfsError::Io("staging lock poisoned".into()))?
            .insert(handle.0, staging);

        Ok(path)
    }

    pub fn seed(&self, handle: VfsHandle, data: &[u8]) -> Result<()> {
        let mut lock = self
            .files
            .lock()
            .map_err(|_| VfsError::Io("staging lock poisoned".into()))?;

        let staging = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;

        staging
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|e| VfsError::Io(format!("staging seek failed: {e}")))?;
        staging
            .file
            .write_all(data)
            .map_err(|e| VfsError::Io(format!("staging seed write failed: {e}")))?;

        Ok(())
    }

    pub fn write_at(&self, handle: VfsHandle, offset: u64, data: &[u8]) -> Result<usize> {
        let mut lock = self
            .files
            .lock()
            .map_err(|_| VfsError::Io("staging lock poisoned".into()))?;

        let staging = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;

        staging
            .file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| VfsError::Io(format!("staging seek failed: {e}")))?;
        staging
            .file
            .write_all(data)
            .map_err(|e| VfsError::Io(format!("staging write failed: {e}")))?;

        Ok(data.len())
    }

    pub fn truncate(&self, handle: VfsHandle, new_size: u64) -> Result<()> {
        let mut lock = self
            .files
            .lock()
            .map_err(|_| VfsError::Io("staging lock poisoned".into()))?;

        let staging = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;

        staging
            .file
            .set_len(new_size)
            .map_err(|e| VfsError::Io(format!("staging truncate failed: {e}")))?;

        Ok(())
    }

    pub fn size(&self, handle: VfsHandle) -> Result<u64> {
        let lock = self
            .files
            .lock()
            .map_err(|_| VfsError::Io("staging lock poisoned".into()))?;

        let staging = lock.get(&handle.0).ok_or(VfsError::InvalidHandle)?;

        staging
            .file
            .metadata()
            .map(|m| m.len())
            .map_err(|e| VfsError::Io(format!("staging stat failed: {e}")))
    }

    pub fn staging_path(&self, handle: VfsHandle) -> Result<PathBuf> {
        let lock = self
            .files
            .lock()
            .map_err(|_| VfsError::Io("staging lock poisoned".into()))?;

        let staging = lock.get(&handle.0).ok_or(VfsError::InvalidHandle)?;
        Ok(staging.path.clone())
    }

    pub fn has_staging(&self, handle: VfsHandle) -> bool {
        self.files
            .lock()
            .map(|l| l.contains_key(&handle.0))
            .unwrap_or(false)
    }

    pub fn read_all(&self, handle: VfsHandle) -> Result<Vec<u8>> {
        let mut lock = self
            .files
            .lock()
            .map_err(|_| VfsError::Io("staging lock poisoned".into()))?;

        let staging = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;

        staging
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|e| VfsError::Io(format!("staging seek failed: {e}")))?;

        let mut buf = Vec::new();
        staging
            .file
            .read_to_end(&mut buf)
            .map_err(|e| VfsError::Io(format!("staging read failed: {e}")))?;

        Ok(buf)
    }

    pub fn read_at(&self, handle: VfsHandle, offset: u64, length: usize) -> Result<Vec<u8>> {
        let mut lock = self
            .files
            .lock()
            .map_err(|_| VfsError::Io("staging lock poisoned".into()))?;

        let staging = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;

        let file_size = staging
            .file
            .metadata()
            .map(|m| m.len())
            .map_err(|e| VfsError::Io(format!("staging stat failed: {e}")))?;

        if offset >= file_size {
            return Ok(Vec::new());
        }

        let to_read = std::cmp::min(length as u64, file_size - offset) as usize;
        let mut buf = vec![0u8; to_read];

        staging
            .file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| VfsError::Io(format!("staging seek failed: {e}")))?;

        let n = staging
            .file
            .read(&mut buf)
            .map_err(|e| VfsError::Io(format!("staging read failed: {e}")))?;
        buf.truncate(n);

        Ok(buf)
    }

    pub fn cleanup(&self, handle: VfsHandle) {
        if let Ok(mut lock) = self.files.lock() {
            if let Some(staging) = lock.remove(&handle.0) {
                let _ = std::fs::remove_file(&staging.path);
            }
        }
    }

    pub fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_staging() -> (StagingManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let mgr = StagingManager::new(dir.path().to_path_buf()).expect("staging");
        (mgr, dir)
    }

    #[test]
    fn write_and_read_back() {
        let (mgr, _dir) = temp_staging();
        let handle = VfsHandle(42);
        mgr.create(handle).expect("create");
        mgr.write_at(handle, 0, b"hello world").expect("write");

        assert_eq!(mgr.size(handle).expect("size"), 11);
        let data = mgr.read_all(handle).expect("read");
        assert_eq!(&data, b"hello world");
    }

    #[test]
    fn write_at_offset() {
        let (mgr, _dir) = temp_staging();
        let handle = VfsHandle(43);
        mgr.create(handle).expect("create");
        mgr.write_at(handle, 0, b"aaaa").expect("write1");
        mgr.write_at(handle, 2, b"BB").expect("write2");

        let data = mgr.read_all(handle).expect("read");
        assert_eq!(&data, b"aaBB");
    }

    #[test]
    fn truncate_shrinks_file() {
        let (mgr, _dir) = temp_staging();
        let handle = VfsHandle(44);
        mgr.create(handle).expect("create");
        mgr.write_at(handle, 0, b"12345678").expect("write");
        mgr.truncate(handle, 3).expect("truncate");

        assert_eq!(mgr.size(handle).expect("size"), 3);
        let data = mgr.read_all(handle).expect("read");
        assert_eq!(&data, b"123");
    }

    #[test]
    fn cleanup_removes_file() {
        let (mgr, _dir) = temp_staging();
        let handle = VfsHandle(45);
        let path = mgr.create(handle).expect("create");
        assert!(path.exists());

        mgr.cleanup(handle);
        assert!(!path.exists());
        assert!(!mgr.has_staging(handle));
    }

    #[test]
    fn read_at_with_offset() {
        let (mgr, _dir) = temp_staging();
        let handle = VfsHandle(46);
        mgr.create(handle).expect("create");
        mgr.write_at(handle, 0, b"abcdefgh").expect("write");

        let data = mgr.read_at(handle, 3, 4).expect("read_at");
        assert_eq!(&data, b"defg");
    }
}

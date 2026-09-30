use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use driftfs_core::FileId;

use crate::error::{Result, VfsError};
use crate::types::{VfsHandle, VfsNodeType};

#[derive(Debug, Clone)]
pub struct HandleEntry {
    pub handle: VfsHandle,
    pub file_id: Option<FileId>,
    pub kind: VfsNodeType,
    pub size: u64,
    pub shortcut_data: Option<Vec<u8>>,
    pub is_dirty: bool,
    pub staging_path: Option<PathBuf>,
    pub last_read_end: Option<u64>,
    pub sequential_count: usize,
}

#[derive(Debug)]
pub struct HandleTable {
    next_id: AtomicU64,
    handles: RwLock<HashMap<u64, HandleEntry>>,
}

impl Default for HandleTable {
    fn default() -> Self {
        Self::new()
    }
}

impl HandleTable {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            handles: RwLock::new(HashMap::new()),
        }
    }

    pub fn allocate(
        &self,
        file_id: Option<FileId>,
        kind: VfsNodeType,
        size: u64,
        shortcut_data: Option<Vec<u8>>,
    ) -> Result<VfsHandle> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let handle = VfsHandle(id);
        let entry = HandleEntry {
            handle,
            file_id,
            kind,
            size,
            shortcut_data,
            is_dirty: false,
            staging_path: None,
            last_read_end: None,
            sequential_count: 0,
        };

        let mut lock = self
            .handles
            .write()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        lock.insert(id, entry);
        Ok(handle)
    }

    pub fn get(&self, handle: VfsHandle) -> Result<HandleEntry> {
        let lock = self
            .handles
            .read()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        lock.get(&handle.0).cloned().ok_or(VfsError::InvalidHandle)
    }

    pub fn release(&self, handle: VfsHandle) -> Result<HandleEntry> {
        let mut lock = self
            .handles
            .write()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        lock.remove(&handle.0).ok_or(VfsError::InvalidHandle)
    }

    pub fn mark_dirty(&self, handle: VfsHandle) -> Result<()> {
        let mut lock = self
            .handles
            .write()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        let entry = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;
        entry.is_dirty = true;
        Ok(())
    }

    pub fn clear_dirty(&self, handle: VfsHandle) -> Result<()> {
        let mut lock = self
            .handles
            .write()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        let entry = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;
        entry.is_dirty = false;
        Ok(())
    }

    pub fn update_size(&self, handle: VfsHandle, new_size: u64) -> Result<()> {
        let mut lock = self
            .handles
            .write()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        let entry = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;
        entry.size = new_size;
        Ok(())
    }

    pub fn set_staging_path(&self, handle: VfsHandle, path: PathBuf) -> Result<()> {
        let mut lock = self
            .handles
            .write()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        let entry = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;
        entry.staging_path = Some(path);
        Ok(())
    }

    pub fn set_file_id(&self, handle: VfsHandle, file_id: FileId) -> Result<()> {
        let mut lock = self
            .handles
            .write()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        let entry = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;
        entry.file_id = Some(file_id);
        Ok(())
    }

    pub fn record_read(&self, handle: VfsHandle, offset: u64, bytes_read: u64) -> Result<usize> {
        let mut lock = self
            .handles
            .write()
            .map_err(|_| VfsError::Io("handle lock poisoned".into()))?;
        let entry = lock.get_mut(&handle.0).ok_or(VfsError::InvalidHandle)?;
        if entry.last_read_end == Some(offset) {
            entry.sequential_count = entry.sequential_count.saturating_add(1);
        } else {
            entry.sequential_count = 1;
        }
        entry.last_read_end = Some(offset.saturating_add(bytes_read));
        Ok(entry.sequential_count)
    }

    pub fn count(&self) -> usize {
        self.handles.read().map(|l| l.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_and_release_handle() {
        let table = HandleTable::new();
        let handle = table
            .allocate(Some(FileId("file_1".into())), VfsNodeType::File, 1024, None)
            .expect("allocate");

        assert_eq!(table.count(), 1);
        let entry = table.get(handle).expect("get");
        assert_eq!(entry.size, 1024);
        assert_eq!(entry.file_id, Some(FileId("file_1".into())));

        let released = table.release(handle).expect("release");
        assert_eq!(released.handle, handle);
        assert_eq!(table.count(), 0);

        assert_eq!(table.get(handle).unwrap_err(), VfsError::InvalidHandle);
    }
}

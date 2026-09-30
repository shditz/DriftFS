use driftfs_filesystem::VfsHandle;
use std::sync::atomic::AtomicBool;
use std::sync::RwLock;

#[derive(Debug)]
pub struct WindowsFileContext {
    pub handle: VfsHandle,
    pub is_directory: bool,
    pub ino: u64,
    path: RwLock<String>,
    pub delete_on_close: AtomicBool,
}

impl WindowsFileContext {
    pub fn new(handle: VfsHandle, is_directory: bool, ino: u64, path: String) -> Self {
        Self {
            handle,
            is_directory,
            ino,
            path: RwLock::new(path),
            delete_on_close: AtomicBool::new(false),
        }
    }

    pub fn path(&self) -> String {
        self.path.read().unwrap().clone()
    }

    pub fn set_path(&self, new_path: String) {
        *self.path.write().unwrap() = new_path;
    }
}

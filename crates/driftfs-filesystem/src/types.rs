use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsNodeType {
    File,
    Directory,
    GoogleDocShortcut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsAttr {
    pub ino: u64,
    pub kind: VfsNodeType,
    pub size: u64,
    pub created_at: Option<SystemTime>,
    pub modified_at: Option<SystemTime>,
    pub mode: u32,
}

impl VfsAttr {
    pub fn directory(ino: u64, modified_at: Option<SystemTime>) -> Self {
        Self {
            ino,
            kind: VfsNodeType::Directory,
            size: 4096,
            created_at: modified_at,
            modified_at,
            mode: 0o755,
        }
    }

    pub fn file(ino: u64, size: u64, modified_at: Option<SystemTime>) -> Self {
        Self {
            ino,
            kind: VfsNodeType::File,
            size,
            created_at: modified_at,
            modified_at,
            mode: 0o644,
        }
    }

    pub fn shortcut(ino: u64, size: u64, modified_at: Option<SystemTime>) -> Self {
        Self {
            ino,
            kind: VfsNodeType::GoogleDocShortcut,
            size,
            created_at: modified_at,
            modified_at,
            mode: 0o444,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsDirEntry {
    pub name: String,
    pub kind: VfsNodeType,
    pub size: u64,
    pub ino: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VfsHandle(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub truncate: bool,
    pub create: bool,
    pub append: bool,
}

impl OpenFlags {
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            truncate: false,
            create: false,
            append: false,
        }
    }

    pub fn write_only() -> Self {
        Self {
            read: false,
            write: true,
            truncate: false,
            create: false,
            append: false,
        }
    }

    pub fn is_read_only(&self) -> bool {
        !self.write && !self.truncate && !self.create && !self.append
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VfsStatFs {
    pub total_space: u64,
    pub free_space: u64,
    pub available_space: u64,
    pub block_size: u32,
}

impl Default for VfsStatFs {
    fn default() -> Self {
        Self {
            total_space: 100 * 1024 * 1024 * 1024,
            free_space: 50 * 1024 * 1024 * 1024,
            available_space: 50 * 1024 * 1024 * 1024,
            block_size: 4096,
        }
    }
}

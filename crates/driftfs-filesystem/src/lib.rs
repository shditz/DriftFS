pub mod error;
pub mod handle_table;
pub mod shortcut;
pub mod staging;
pub mod types;
pub mod vfs;

pub use error::{Result, VfsError};
pub use handle_table::{HandleEntry, HandleTable};
pub use shortcut::GoogleWorkspaceShortcut;
pub use staging::StagingManager;
pub use types::{OpenFlags, VfsAttr, VfsDirEntry, VfsHandle, VfsNodeType, VfsStatFs};
pub use vfs::{file_id_to_ino, DriftFsVfs, ROOT_INODE};

# `driftfs-filesystem`

Platform-agnostic virtual filesystem (VFS) interface, path resolver, staging coordinator, and Google Workspace shortcut generator for DriftFS.

## Scope

- **Platform-Agnostic VFS Core**: Implements virtual filesystem logic decoupling platform adapters (WinFsp on Windows, FUSE 3 on Linux, macFUSE on macOS) from cloud storage details.
- **Path and Inode Resolution**: Maps 64-bit filesystem inodes to remote `FileId` records, maintaining deterministic inode generation (`file_id_to_ino`) and fast reverse lookup.
- **Staging Coordinator**: Intercepts write and truncate calls, buffering modifications in local temporary staging files before uploading to Google Drive upon file close.
- **Workspace Shortcuts**: Intercepts Google Docs, Sheets, and Slides entries, dynamically presenting them as synthetic `.url` Internet Shortcuts pointing to document edit URLs.
- **Handle Table**: Tracks open file descriptors, read/write flags, and cursor positions safely across threads.

## Primary Exports

- `DriftFsVfs`: Primary virtual filesystem facade.
- `StagingManager`: Coordinates local staging files and crash recovery.
- `GoogleWorkspaceShortcut`: Parses Google Workspace MIME types and generates `.url` payloads.
- `VfsAttr`, `VfsDirEntry`, `VfsNodeType`, `VfsStatFs`: Cross-platform filesystem metadata structures.
- `VfsHandle`, `HandleTable`: File handle management primitives.
- `ROOT_INODE`: Inode constant (`1`) representing the root directory.

## Example

```rust
use driftfs_filesystem::{DriftFsVfs, ROOT_INODE};
use std::sync::Arc;

// Instantiate VFS backed by provider, metadata store, and chunk cache
let vfs = Arc::new(DriftFsVfs::new(provider, metadata_store, chunk_cache));

// Look up root attributes or directory entries
let root_attr = vfs.getattr(ROOT_INODE).await.expect("getattr root");
println!("Root inode: {}, kind: {:?}", root_attr.ino, root_attr.kind);
```

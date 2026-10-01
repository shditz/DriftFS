# `driftfs-provider`

Abstract cloud storage provider interface, data transfer objects, and change stream definitions for DriftFS.

## Scope

- **`CloudProvider` Trait**: Defines asynchronous operations for cloud backends (metadata retrieval, directory enumeration, byte-range reading, multipart/resumable file uploads, object renaming, move, trash, delete, and incremental change feeds).
- **Core Models**: `ObjectMetadata`, `ObjectKind`, `Change`, `ChangePage`, and `ReadOutput`.
- **Decoupling**: Allows filesystem, sync, and caching layers to operate against mock providers or alternative storage backends without coupling to Google Drive APIs.

## Primary Exports

- `CloudProvider`: Asynchronous trait implemented by concrete storage providers.
- `ObjectMetadata`: Unified object record containing file ID, name, parent ID, size, MIME type, and timestamps.
- `ObjectKind`: Enum distinguishing `File` and `Directory`.
- `Change`: Delta event representing `Upsert(ObjectMetadata)` or `Delete { id }`.
- `ChangePage`: Batch of changes along with the next pagination checkpoint token.
- `ReadOutput`: Payload bytes matched with the requested `ByteRange`.
- `SessionCreatedHook`: Callback closure for tracking resumable upload session URIs.

## Example

```rust
use driftfs_core::{ByteRange, FileId, Result};
use driftfs_provider::{CloudProvider, ObjectMetadata};

async fn inspect_remote_file<P: CloudProvider>(provider: &P, id: &FileId) -> Result<ObjectMetadata> {
    let meta = provider.get_metadata(id).await?;
    println!("File name: {}, size: {:?} bytes", meta.name, meta.size_bytes);
    Ok(meta)
}
```

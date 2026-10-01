# `driftfs-cache`

Bounded local chunk cache with LRU eviction and memory-mapped disk persistence for DriftFS.

## Scope

- **Fixed-Size Chunking**: Chunks remote files into uniform 1 MB blocks (`DEFAULT_CHUNK_SIZE = 1048576`).
- **LRU Eviction**: Tracks chunk access frequency and timestamps. When cache capacity exceeds configured thresholds, oldest chunks are evicted automatically.
- **Disk Backing**: Stores chunks under `%LOCALAPPDATA%\driftfs\cache\` (or custom path) keyed by file ID, remote version, and chunk index (`ChunkKey`).
- **Atomic Writes**: Writes chunk files with temporary file replacements to avoid corruption during mid-write crashes.
- **Selective Invalidation**: Supports fine-grained invalidation by individual file ID or remote version tag.

## Primary Exports

- `BoundedChunkCache`: High-level thread-safe chunk cache manager.
- `ChunkKey`: Identifier composed of `FileId`, version string, and chunk index.
- `DEFAULT_CHUNK_SIZE`: Default chunk size constant (1 MiB / 1,048,576 bytes).
- `LruTracker`: In-memory LRU tracking data structure.
- `CacheError`: Cache operations error enum.

## Example

```rust
use driftfs_cache::{BoundedChunkCache, ChunkKey, DEFAULT_CHUNK_SIZE};
use driftfs_core::FileId;
use std::path::PathBuf;

let cache = BoundedChunkCache::new(
    PathBuf::from("./cache"),
    512 * 1024 * 1024, // 512 MB limit
    DEFAULT_CHUNK_SIZE,
).expect("init cache");

let key = ChunkKey::new(FileId("1A2B3C".into()), "v1", 0);
// Reads or writes chunks asynchronously
```

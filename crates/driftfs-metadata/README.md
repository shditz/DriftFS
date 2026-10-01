# `driftfs-metadata`

Embedded SQLite metadata index, hierarchy repository, and crash-resilient write staging journal for DriftFS.

## Scope

- **Local Metadata Cache**: Caches directory trees, object attributes, and sync states in SQLite to serve filesystem directory queries with sub-millisecond latency.
- **WAL Mode & Pragmas**: Enforces `PRAGMA journal_mode = WAL;`, `PRAGMA synchronous = NORMAL;`, and `PRAGMA busy_timeout = 5000;`.
- **Atomic Sync Checkpoints**: Commits object upserts/deletions and sync pagination tokens in a single transaction to guarantee crash consistency.
- **Name Collision Handling**: Disambiguates identical file names in the same folder with ` (1)` local suffixes while preserving `remote_name`.
- **Write Staging Journal**: Manages `staging_journal` records for local file edits and resumable upload recovery.

## Primary Exports

- `MetadataStore`: Primary metadata repository struct.
- `StoredObject`: Database record representation for cached objects.
- `StagingJournalEntry`, `StagingState`: Journal records for staged uploads.
- `SyncStatus`: Status enum (`synced`, `pending_upload`, `pending_delete`).
- `MetadataError`: Error types for database and migration operations.

## Example

```rust
use driftfs_metadata::MetadataStore;
use std::path::Path;

let store = MetadataStore::open(Path::new("metadata.db")).expect("open database");
let root_items = store.list_children(None).expect("list root");

for item in root_items {
    println!("Item: {} ({:?})", item.name, item.kind);
}
```

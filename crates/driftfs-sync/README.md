# `driftfs-sync`

Incremental change synchronization engine and background worker for DriftFS.

## Scope

- **Incremental Change Sync**: Consumes Google Drive change streams via `Changes: list` using pagination page tokens (`startPageToken`).
- **Atomic State Updates**: Applies upserts and deletions to the SQLite metadata store and advances checkpoints inside single transactions.
- **Cache Invalidation**: Triggers eviction of cached file chunks in `driftfs-cache` when remote file modifications or deletions occur.
- **Background Worker**: Manages a long-running asynchronous worker loop with cancellation tokens, configurable poll intervals, and fault recovery.
- **Directory On-Demand Sync**: Syncs child items when a user navigates to an unindexed folder.

## Primary Exports

- `SyncEngine`: Synchronizes remote provider state to local metadata store.
- `SyncWorker`: Asynchronous background task running periodic polling loops.
- `SyncError`: Errors during sync execution.

## Example

```rust
use driftfs_sync::{SyncEngine, SyncWorker};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// Instantiate engine and run background worker
let engine = Arc::new(SyncEngine::new(provider, metadata_store, account_id));
let cancel = CancellationToken::new();
let worker = SyncWorker::new(engine, Duration::from_secs(60), cancel.clone());

tokio::spawn(async move {
    worker.run().await;
});
```

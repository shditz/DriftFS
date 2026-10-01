# Database Schema & Migrations

DriftFS uses SQLite as an embedded, low-latency metadata store. It tracks remote object hierarchies, sync checkpoints, on-demand directory sync states, and crash-resilient write staging operations.

Database file location on Windows:
```
%LOCALAPPDATA%\DriftFS\metadata.db
```

## Pragmas & Concurrency Configuration

Every database connection is initialized with these pragmas:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

### Rationale

- **WAL (Write-Ahead Logging)**: Allows concurrent readers to operate without blocking writes from background synchronization workers or filesystem mutations.
- **synchronous = NORMAL**: Provides crash durability across application crashes while avoiding redundant disk flushes on every SQLite transaction commit.
- **foreign_keys = ON**: Enforces relational consistency across references.
- **busy_timeout = 5000**: Waits up to 5000 milliseconds when acquiring locks before returning `SQLITE_BUSY`.

---

## Schema Tables

### 1. `schema_migrations`

Tracks schema migration versions applied to the database instance.

```sql
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
```

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `version` | `INTEGER` | `PRIMARY KEY` | Monotonically increasing migration version. |
| `applied_at` | `TEXT` | `NOT NULL` | UTC ISO 8601 timestamp when migration ran. |

---

### 2. `objects`

Stores indexed metadata for remote files and folders discovered from Google Drive.

```sql
CREATE TABLE IF NOT EXISTS objects (
    id TEXT PRIMARY KEY,
    parent_id TEXT,
    name TEXT NOT NULL COLLATE NOCASE,
    remote_name TEXT NOT NULL,
    kind TEXT NOT NULL,
    size_bytes INTEGER,
    mime_type TEXT,
    created_at TEXT,
    modified_at TEXT,
    version TEXT,
    sync_status TEXT NOT NULL DEFAULT 'synced',
    deleted INTEGER NOT NULL DEFAULT 0
);
```

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | `TEXT` | `PRIMARY KEY` | Authoritative remote object identifier (Google Drive file ID). |
| `parent_id` | `TEXT` | Nullable | Remote ID of the parent folder (`NULL` for root directory items). |
| `name` | `TEXT` | `COLLATE NOCASE` | Sanitized local display name. Disambiguated with ` (1)` suffix on collisions. |
| `remote_name` | `TEXT` | `NOT NULL` | Original object name on Google Drive. |
| `kind` | `TEXT` | `NOT NULL` | Object type: `'file'` or `'folder'`. |
| `size_bytes` | `INTEGER` | Nullable | File payload size in bytes (`NULL` for folders). |
| `mime_type` | `TEXT` | Nullable | Content MIME type. |
| `created_at` | `TEXT` | Nullable | UTC ISO 8601 creation timestamp. |
| `modified_at` | `TEXT` | Nullable | UTC ISO 8601 modification timestamp. |
| `version` | `TEXT` | Nullable | Remote version / generation tag. |
| `sync_status` | `TEXT` | `DEFAULT 'synced'` | Sync state: `'synced'`, `'pending_upload'`, `'pending_delete'`. |
| `deleted` | `INTEGER` | `DEFAULT 0` | Soft-deletion flag (`0` active, `1` tombstoned). |

#### Indexes

- `idx_objects_parent`: `(parent_id, deleted)` for listing directory contents.
- `idx_objects_lookup`: `(parent_id, name COLLATE NOCASE, deleted)` for case-insensitive path resolution.
- `idx_objects_deleted`: `(deleted)` for tombstone cleanup queries.
- `idx_objects_parent_sorted`: `(parent_id, deleted, kind DESC, name COLLATE NOCASE ASC)` for sorted directory enumeration.

---

### 3. `sync_checkpoints`

Tracks the incremental change sync token for authenticated accounts.

```sql
CREATE TABLE IF NOT EXISTS sync_checkpoints (
    account_id TEXT PRIMARY KEY,
    checkpoint_token TEXT NOT NULL,
    last_sync_at TEXT NOT NULL
);
```

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `account_id` | `TEXT` | `PRIMARY KEY` | Account identifier string. |
| `checkpoint_token` | `TEXT` | `NOT NULL` | Provider pagination page token (`startPageToken`). |
| `last_sync_at` | `TEXT` | `NOT NULL` | UTC ISO 8601 timestamp of the last successful sync cycle. |

---

### 4. `synced_directories`

Maintains the set of directories whose contents have been fetched and synced from the provider.

```sql
CREATE TABLE IF NOT EXISTS synced_directories (
    dir_id TEXT PRIMARY KEY,
    synced_at TEXT NOT NULL
);
```

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `dir_id` | `TEXT` | `PRIMARY KEY` | Directory remote identifier. |
| `synced_at` | `TEXT` | `NOT NULL` | UTC ISO 8601 timestamp when directory contents were enumerated. |

---

### 5. `staging_journal`

Write-ahead journal for file uploads and local staging files. Guarantees crash recovery across unexpected process termination.

```sql
CREATE TABLE IF NOT EXISTS staging_journal (
    handle_id INTEGER PRIMARY KEY,
    file_id TEXT NOT NULL,
    staging_path TEXT NOT NULL,
    parent_id TEXT,
    state TEXT NOT NULL DEFAULT 'staging',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    session_uri TEXT,
    uploaded_bytes INTEGER DEFAULT 0
);
```

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `handle_id` | `INTEGER` | `PRIMARY KEY` | Unique runtime staging handle identifier. |
| `file_id` | `TEXT` | `NOT NULL` | Target object identifier. |
| `staging_path` | `TEXT` | `NOT NULL` | Absolute filesystem path to local staging file. |
| `parent_id` | `TEXT` | Nullable | Target parent folder identifier for newly created files. |
| `state` | `TEXT` | `DEFAULT 'staging'` | Journal status: `'staging'`, `'ready_to_upload'`, `'uploading'`, `'completed'`, `'failed'`. |
| `created_at` | `INTEGER` | `NOT NULL` | Epoch timestamp in milliseconds. |
| `updated_at` | `INTEGER` | `NOT NULL` | Epoch timestamp in milliseconds. |
| `session_uri` | `TEXT` | Nullable | Resumable upload session URI returned by Google Drive API. |
| `uploaded_bytes` | `INTEGER` | `DEFAULT 0` | Byte count uploaded in current resumable upload session. |

#### Indexes

- `idx_staging_journal_file_id`: `(file_id)` for quick lookup during open handles and crash recovery.

---

## Migration History

Schema migrations execute automatically inside SQLite transactions when `MetadataStore::open(path)` is called.

### Version 1 (Phase 2)
- Created `objects` table and primary lookup indexes.
- Created `sync_checkpoints` table.
- Added record into `schema_migrations`.

### Version 2 (Phase 2 & Phase 3)
- Created `synced_directories` table to support on-demand directory sync.

### Version 3 (Phase 4)
- Created `staging_journal` table for crash-consistent file write staging.
- Created `idx_staging_journal_file_id` index.

### Version 4 (Phase 7)
- Created `idx_objects_parent_sorted` for fast, sorted directory enumerations.
- Added `session_uri` column to `staging_journal` for resumable upload recovery.
- Added `uploaded_bytes` column to `staging_journal`.

---

## Crash Consistency Guarantees

1. **Transactional Batch Sync**:
   During background incremental sync, batch upserts/deletions in `objects` and the update to `sync_checkpoints` are committed in the same SQLite transaction. If the application crashes midway, the database rolls back to the prior checkpoint.

2. **Orphan Normalization**:
   On initialization, `MetadataStore::open` runs `normalize_root_orphans()`, resetting orphaned child records with nonexistent parents to root (`parent_id = NULL`).

3. **Staging Recovery**:
   On startup, filesystem initialization scans `staging_journal` entries. Incomplete uploads in `'ready_to_upload'` or `'uploading'` states are re-queued or safely cleaned up if orphaned.

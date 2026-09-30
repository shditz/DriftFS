# Changelog

All notable changes to DriftFS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-09-30

### Added
- **Core Domain Primitives (`driftfs-core`)**: Strongly typed `FileId`, `AccountId`, `ByteRange`, input path sanitization preventing traversal, and unified `DriftFsError` enum.
- **Secure Authentication & Keyring (`driftfs-auth`)**: RFC 7636 OAuth 2.0 PKCE flow, loopback TCP redirect listener with state parameter validation, and OS keyring storage via Windows Credential Manager, Secret Service, and macOS Keychain.
- **Google Drive API v3 Provider (`driftfs-google-drive`)**: Resumable multipart uploads, chunked media streaming with Range headers, exponential backoff with jitter on HTTP 429/5xx status codes, and JSON parsing.
- **SQLite Metadata Hierarchy (`driftfs-metadata`)**: Persistent hierarchy repository with WAL mode (`PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;`), parent-child index, parent transaction consistency, and automatic local name collision disambiguation (` (1)` suffix).
- **Incremental Sync Engine (`driftfs-sync`)**: Periodic polling worker consuming Google Drive change tokens, advancing metadata transactions atomically with `sync_checkpoints`.
- **Windows Filesystem Adapter (`driftfs-platform-windows`)**: User-mode virtual filesystem powered by WinFsp, providing on-demand directory queries, direct read dispatch, file attribute mapping, and file handles.
- **Write & Mutation Pipeline (`driftfs-filesystem`)**: Persistent write-ahead staging journal, atomic file creation, truncation, streaming file write flush, remote deletion, and cross-directory move/rename synchronization.
- **Bounded Chunk Cache (`driftfs-cache`)**: LRU eviction engine maintaining configurable disk footprints (default: 512 MB), async write-back flushing, and predictive chunk prefetching for sequential read streams.
- **Desktop GUI & System Tray (`driftfs-ui`)**: Native desktop control panel built with Slint and `tray-icon`, featuring live mount toggling, quota visualization, cache purge actions, and activity logs.
- **Hardening & Fault Injection (`driftfs-testkit`)**: In-memory `MockProvider`, deterministic network latency injection, token revocation simulations, multi-platform platform adapter skeletons for Linux (FUSE 3) and macOS (macFUSE), and automatic crash recovery reconciliation on boot.
- **Release Automation & Packaging**: Windows Inno Setup installer script with WinFsp runtime registry detection, PowerShell packaging script generating portable zip archives with SHA256 checksums, and GitHub Actions multi-platform release CI workflow.

### Changed
- **Timestamp Parsing Extraction (`driftfs-core`)**: Extracted ISO-8601 calendar arithmetic from `driftfs-filesystem` into modular `driftfs-core::time` module with dedicated unit tests.
- **Cache Mutex Error Harmonization (`driftfs-cache`)**: Replaced direct mutex unwrap calls in `BoundedChunkCache` with typed `CacheError::LockPoisoned` error propagation matching workspace conventions.
- **Concurrent Cache Reservation (`driftfs-cache`)**: Introduced in-flight capacity reservation in `LruTracker` and `BoundedChunkCache::put` to prevent temporary disk footprint overshoots during high-concurrency prefetch operations.

# DriftFS Test Suites

## Architecture of Tests

DriftFS organizes automated tests across three levels:

1. **Crate Unit & Contract Tests:** Collocated within each crate's `src/` directory under `#[cfg(test)]` modules. These test isolated logic (e.g. byte range arithmetic in `driftfs-core`, token lifecycle in `driftfs-auth`, pragma and collision handling in `driftfs-metadata`, sync engine loop in `driftfs-sync`, cache watermark eviction in `driftfs-cache`, and UI controller state in `driftfs-ui`).
2. **Deterministic Mock Providers:** Located in `crates/driftfs-testkit`. Provides `MockProvider` to simulate Google Drive API calls, pagination, mutation operations, and change streams without real network calls.
3. **End-to-End VFS & Adapter Integration Tests:** 
   - `crates/driftfs-filesystem/tests/vfs_test.rs`: 25 integration tests verifying file/folder creation, truncation, streaming reads, write-at-offset, rename, cross-directory moves, rmdir, and Google Doc shortcut generation.
   - `crates/driftfs-filesystem/tests/torture_test.rs`: Filesystem stress and edge-case testing under intensive random mutation and read cycles.
   - `crates/driftfs-filesystem/tests/security_hardening_test.rs`: Path traversal prevention, illegal character sanitization, and permission boundary validation.
   - `crates/driftfs-sync/tests/concurrency_test.rs`: Multi-threaded metadata concurrency test verifying WAL mode consistency.
   - `platforms/windows/tests/adapter_test.rs`: Integration tests for WinFsp callback dispatching, mutation flows, and a live Windows WinFsp mount test.

## Running Tests

Execute all workspace tests (112 tests across all workspace crates):

```bash
cargo test --all
```

To run tests for a specific crate:

```bash
cargo test -p driftfs-filesystem
cargo test -p driftfs-platform-windows
cargo test -p driftfs-metadata
cargo test -p driftfs-sync
cargo test -p driftfs-auth
cargo test -p driftfs-cache
cargo test -p driftfs-ui
```

See [docs/architecture.md](../docs/architecture.md) for architectural details and test coverage patterns.

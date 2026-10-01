# `driftfs-core`

Shared foundational domain primitives, newtype identifiers, input sanitization routines, time utilities, and error types across the DriftFS monorepo.

## Scope

- **Domain Identifiers**: Strongly-typed newtypes (`FileId`, `AccountId`, `MountId`, `ProviderId`, `ByteRange`) preventing stringly-typed bugs.
- **Sanitizer**: Validates file names and sanitizes paths across Windows, Linux, and macOS. Replaces illegal characters (`<`, `>`, `:`, `"`, `/`, `\`, `|`, `?`, `*`) and detects Windows reserved names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`).
- **Time Utilities**: ISO 8601 parsing and timestamp formatting.
- **Error Model**: Central `DriftFsError` enum defining unified filesystem, provider, authorization, and storage errors.

## Primary Exports

- `FileId`, `AccountId`, `MountId`, `ProviderId`: Authoritative identity wrappers.
- `ByteRange`: `ByteRange { start: u64, end: Option<u64> }` representing HTTP range request boundaries.
- `sanitize_path`, `validate_file_name`, `is_windows_reserved`: Cross-platform path validation.
- `DriftFsError`, `Result<T>`: Workspace error primitives.

## Example

```rust
use driftfs_core::{validate_file_name, ByteRange, FileId};

let file_id = FileId("1A2B3C4D5E".into());
let range = ByteRange::new(0, Some(1048576));

assert!(validate_file_name("valid_doc.pdf").is_ok());
assert!(validate_file_name("invalid:name.txt").is_err());
```

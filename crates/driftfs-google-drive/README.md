# `driftfs-google-drive`

Google Drive API v3 HTTP client and `CloudProvider` implementation with exponential backoff and resumable uploads for DriftFS.

## Scope

- **`CloudProvider` Implementation**: Translates virtual filesystem calls into Google Drive API v3 requests.
- **Resumable Uploads**: Supports large file uploads via Google Drive resumable upload sessions, checkpointing uploaded byte offsets for network interruption recovery.
- **HTTP Range Requests**: Implements partial content downloads (`Range: bytes=start-end`) for on-demand streaming reads.
- **Rate Limiting & Retries**: Implements exponential backoff with jitter on HTTP 429 (Too Many Requests) and 5xx transient server errors.
- **Workspace Document Mapping**: Maps Google Docs, Sheets, and Slides MIME types to synthetic `.url` web shortcuts.

## Primary Exports

- `GoogleDriveProvider`: Concrete implementation of `driftfs_provider::CloudProvider`.
- `DriveClientConfig`: Client timeout, backoff, and retry policy configuration.
- `drive_file_to_metadata`: Deserialization helper mapping Google Drive JSON representations to `ObjectMetadata`.
- `DriveAbout`, `DriveStorageQuota`: Account profile and storage quota data structures.

## Example

```rust
use driftfs_auth::TokenProvider;
use driftfs_google_drive::GoogleDriveProvider;
use std::sync::Arc;

fn create_drive_provider(tokens: Arc<dyn TokenProvider>) -> GoogleDriveProvider {
    GoogleDriveProvider::new(tokens)
}
```

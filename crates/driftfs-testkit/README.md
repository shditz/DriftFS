# `driftfs-testkit`

In-memory mock cloud provider, fault injection engine, and deterministic test fixtures for DriftFS.

## Scope

- **`MockProvider`**: In-memory thread-safe implementation of `driftfs_provider::CloudProvider` simulating Google Drive hierarchy, byte streams, and change feeds without external network calls.
- **`FaultInjector`**: Programmable chaos engineering component simulating network drops, HTTP 429 rate limiting, token expiration, mid-stream read cutoffs, server errors, and upload failures.
- **Fixtures**: Helper functions for constructing deterministic file and folder metadata (`create_metadata`).

## Primary Exports

- `MockProvider`: In-memory cloud provider.
- `FaultInjector`, `FaultRule`, `FaultType`: Fault injection primitives.
- `create_metadata`: Deterministic `ObjectMetadata` test builder.

## Example

```rust
use driftfs_core::{ByteRange, FileId};
use driftfs_provider::CloudProvider;
use driftfs_testkit::MockProvider;

#[tokio::test]
async fn test_rate_limit_resilience() {
    let mock = MockProvider::new();
    mock.add_directory("root", "My Drive", None);
    mock.inject_rate_limit("get_metadata", 0, 1, Some(30));

    let err = mock.get_metadata(&FileId("root".into())).await;
    assert!(err.is_err());
}
```

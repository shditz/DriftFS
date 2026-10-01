# `driftfs-logging`

Structured tracing initialization, log subscriber configuration, and secret token redaction routines for DriftFS.

## Scope

- **Tracing Subscriber**: Initializes `tracing-subscriber` with configurable log levels or directives.
- **Environment Overrides**: Reads `DRIFTFS_LOG` to override configured log filters at runtime.
- **Secret Redaction**: Prevents sensitive credentials (OAuth tokens, refresh tokens, client secrets) from leaking into terminal outputs or log files.

## Primary Exports

- `init(filter_directive: &str)`: Configures and registers the global tracing subscriber.
- `redact(value: &str) -> String`: Sanitizes secret strings into `"[REDACTED]"`.

## Example

```rust
use driftfs_logging::{init, redact};

// Initialize logging with "info" filter directive
init("driftfs=debug,info");

let secret_token = "ya29.a0AfH6SMB...";
tracing::info!(token = %redact(secret_token), "Authenticated successfully");
```

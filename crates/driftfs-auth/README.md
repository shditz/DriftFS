# `driftfs-auth`

OAuth 2.0 PKCE authentication flow, loopback TCP redirect server, operating system keyring storage, and account registry management for DriftFS.

## Scope

- **OAuth 2.0 PKCE Flow**: Generates cryptographic code verifiers and SHA-256 challenges, opens system browser, and binds to an ephemeral loopback TCP port (`127.0.0.1:0`) to capture authorization codes.
- **Secure Keyring Storage**: Stores sensitive refresh and access tokens in OS keyrings (Windows Credential Manager, macOS Keychain, Linux Secret Service) using `keyring-rs`. Never stores plaintext tokens on disk.
- **Zero Plaintext Secrets**: Wraps credentials in `SecretToken` with custom `Debug` implementations to prevent log leakage.
- **Account Registry**: Manages non-sensitive account metadata (display name, email, account ID) in `%APPDATA%\DriftFS\accounts.json`.
- **Token Refresh Lifecycle**: Proactively refreshes expiring access tokens before dispatching API requests.

## Primary Exports

- `AuthService`: High-level authentication orchestrator.
- `GoogleOAuthClient`: OAuth 2.0 implementation with PKCE challenge generation and token exchange.
- `KeyringCredentialStore`: Production credential store backing onto the OS keyring.
- `InMemoryCredentialStore`: Volatile credential store for automated tests.
- `AccountStore`, `AccountRegistry`: Local account metadata storage.
- `SecretToken`: Secure string wrapper preventing accidental credential logging.
- `get_build_time_default_client_id()`: Retrieves baked-in OAuth Client ID when configured at build time.

## Example

```rust
use driftfs_auth::{AuthService, KeyringCredentialStore, OAuthConfig};
use std::sync::Arc;

let oauth_config = OAuthConfig {
    client_id: "example-client-id.apps.googleusercontent.com".into(),
    client_secret: None,
};

let store = Arc::new(KeyringCredentialStore::new());
let auth = AuthService::new(oauth_config, store);
```

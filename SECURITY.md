# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in DriftFS, please report it responsibly.

**Do not open a public issue.**

Send a detailed report to the project maintainers via email or private GitHub Security Advisory. Include:

1. Description of the vulnerability.
2. Steps to reproduce.
3. Potential impact.
4. Suggested fix (if any).

We will acknowledge your report within 48 hours and work toward a fix. You will be credited in the release notes unless you prefer anonymity.

## Security Scope

DriftFS handles the following security-sensitive operations:

- **OAuth 2.0 tokens**: Access and refresh tokens are stored exclusively in the OS credential manager (Windows Credential Manager). They are never written to disk in plaintext.
- **PKCE authorization flow**: Authorization codes are exchanged using SHA-256 challenges per RFC 7636.
- **Secret redaction**: The `SecretToken` type redacts all sensitive values in debug output and log streams.
- **Local metadata**: SQLite databases contain file metadata (names, IDs, sizes) but never file contents or credentials.

## Supported Versions

Security fixes are applied to the latest development branch. There are no stable release versions yet.

## Known Trust Boundaries

- User input from the OS filesystem layer (file paths, byte ranges).
- HTTP responses from Google Drive API.
- OAuth redirect callbacks on the local loopback interface.
- OS keyring access for credential retrieval.

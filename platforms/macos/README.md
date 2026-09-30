# macOS Platform Adapter (`driftfs-platform-macos`)

> **Status: Early Scaffolding**  
> The crate structure and mount configuration models are implemented in the workspace. Native macFUSE callback bindings are planned for upcoming releases.

Filesystem adapter that will expose the DriftFS virtual filesystem to macOS (e.g. `/Volumes/DriftFS/GoogleDrive`).

## Prerequisites

- macOS 12 (Monterey) or newer
- Xcode Command Line Tools (`xcode-select --install`)
- macFUSE or native FileProvider framework support

## Architecture

```mermaid
graph LR
    Finder[Finder / macOS Applications] --> MacOSVFS[macOS VFS Layer]
    MacOSVFS --> Adapter[macOS Platform Adapter]
    Adapter --> VFS[driftfs-filesystem]
```

See [docs/architecture.md](../../docs/architecture.md) for architectural scope and design details.

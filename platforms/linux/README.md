# Linux Platform Adapter (`driftfs-platform-linux`)

> **Status: Early Scaffolding**  
> The crate structure and mount configuration models are implemented in the workspace. Native FUSE 3 callback bindings are planned for upcoming releases.

FUSE-based filesystem adapter that will expose the DriftFS virtual filesystem to Linux as a native mount point directory (e.g. `~/DriftFS/GoogleDrive`).

## Prerequisites

- Linux kernel 5.4+ with FUSE support
- `libfuse3-dev` (Debian/Ubuntu) or `fuse3-devel` (Fedora/RHEL/Arch)
- GCC / Clang C++ compiler toolchain

## Architecture

```mermaid
graph LR
    Apps[Linux Applications / VFS] --> FUSEKernel[Linux FUSE Kernel Module]
    FUSEKernel --> LibFUSE[libfuse3]
    LibFUSE --> LinuxPlatform[driftfs-platform-linux]
    LinuxPlatform --> VFS[driftfs-filesystem]
```

See [docs/architecture.md](../../docs/architecture.md) for architectural scope and design details.

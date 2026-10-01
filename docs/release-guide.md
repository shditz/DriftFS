# Release Maintainer Guide

This document defines the release lifecycle, versioning policy, and verification procedures for DriftFS maintainers.

---

## 1. Versioning Policy

DriftFS strictly follows [Semantic Versioning (SemVer 2.0.0)](https://semver.org/):

- **Major (X.0.0)**: Incompatible architectural shifts, database schema migrations without automatic rollback, or major breaking VFS API changes.
- **Minor (0.X.0)**: New functional capabilities, new platform adapters, or backwards-compatible protocol enhancements.
- **Patch (0.0.X)**: Bug fixes, security patches, performance improvements, and documentation updates.

---

## 2. Pre-Release Verification Gate

Before tagging any release, you must execute and verify all three checks locally:

```powershell
# 1. Format Check
cargo fmt --all -- --check

# 2. Static Analysis Linting
cargo clippy --all-targets

# 3. Test Suite (all 109 tests across workspace crates)
cargo test --all
```

All three commands must exit with status code `0` with zero warnings and zero test failures.

---

## 3. Version Bump Checklist

Update the version number across the following files:

1. **Workspace Definition** (`Cargo.toml`):
   ```toml
   [workspace.package]
   version = "1.0.0"
   ```
2. **Windows Installer Script** (`installer/windows/driftfs.iss`):
   ```pascal
   #define MyAppVersion "1.0.0"
   ```
3. **Changelog** (`CHANGELOG.md`):
   - Update `[Unreleased]` to the target version and current date (e.g. `## [1.0.0] - 2026-10-01`).
   - Ensure all changes are categorized under `Added`, `Changed`, `Fixed`, or `Removed`.

---

## 4. Build-Time OAuth Client ID Injection

For official production releases, you can inject a default Google Cloud OAuth Client ID during compilation so end-users do not need to create their own Google Cloud Project:

```powershell
$env:DRIFTFS_DEFAULT_CLIENT_ID = "your-client-id.apps.googleusercontent.com"
cargo build --release -p driftfs-ui
```

When unset, the application allows users to supply custom credentials via the Settings tab or `config.toml`.

---

## 5. Local Packaging & Checksum Verification

Run the packaging script to generate the Windows release payload:

```powershell
.\scripts\package.ps1 -Version 0.1.0
```

Verify that the following artifacts are produced in `dist/`:

- `dist/driftfs-v0.1.0-windows-x86_64.zip`
- `dist/driftfs-setup-v0.1.0-windows-x86_64.exe` (if Inno Setup is installed)
- `dist/SHA256SUMS.txt`

Verify the generated checksum manually:

```powershell
Get-FileHash .\dist\driftfs-v0.1.0-windows-x86_64.zip -Algorithm SHA256
```

---

## 6. Tagging & CI/CD Deployment

1. Commit all version bumps and changelog updates:
   ```bash
   git add Cargo.toml Cargo.lock installer/windows/driftfs.iss CHANGELOG.md
   git commit -m "chore: prepare release v0.1.0"
   ```

2. Create an annotated git tag:
   ```bash
   git tag -a v0.1.0 -m "Release v0.1.0"
   ```

3. Push the tag to upstream:
   ```bash
   git push origin main
   git push origin v0.1.0
   ```

4. The GitHub Actions release workflow (`.github/workflows/release.yml`) triggers automatically:
   - Compiles Windows x86_64 release binary and Inno Setup installer.
   - Generates SHA256 checksums manifest.
   - Publishes a GitHub Release containing Windows zip and setup installer assets (Unix packaging pipelines will be enabled with native FUSE releases).

5. Inspect the release in GitHub Releases, review release notes against `CHANGELOG.md`, and publish.

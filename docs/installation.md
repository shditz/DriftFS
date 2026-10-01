# Installation Guide

This guide covers installing and configuring DriftFS on Windows, Linux, and macOS.

---

## System Requirements

| Platform | Minimum OS | Required Dependencies |
| :--- | :--- | :--- |
| **Windows** | Windows 10 / 11 (64-bit) | [WinFsp](https://winfsp.dev/rel/) (2023 or newer) |
| **Linux** | Ubuntu 22.04+, Debian 12+, Fedora 38+ | `fuse3`, `libfuse3-dev` |
| **macOS** | macOS 12 (Monterey) or newer | [macFUSE](https://osxfuse.github.io/) |

---

## Windows Installation

### Option 1: Windows Installer (Recommended)

1. Download the latest installer `driftfs-setup-v1.0.0-windows-x86_64.exe` from the [GitHub Releases](https://github.com/shditz/DriftFS/releases).
2. Run the installer. The setup wizard automatically checks for the WinFsp runtime:
   - If WinFsp is missing, the installer prompts to open the official WinFsp download page.
   - Install WinFsp, then complete the DriftFS setup wizard.
3. Launch **DriftFS** from the Start Menu or Desktop shortcut.

### Option 2: Portable Zip Archive

1. Download `driftfs-v1.0.0-windows-x86_64.zip` from [GitHub Releases](https://github.com/shditz/DriftFS/releases).
2. Ensure [WinFsp](https://winfsp.dev/rel/) is installed on your system. You can install it using `winget`:
   ```powershell
   winget install -e --id WinFsp.WinFsp
   ```
3. Extract the zip archive to a directory of your choice (e.g. `C:\Tools\DriftFS`).
4. Execute `driftfs-ui.exe`.

### Option 3: Build from Source

Prerequisites:
- [Rust toolchain](https://rustup.rs/) (1.75+ recommended)
- [WinFsp developer packages](https://winfsp.dev/rel/)

```powershell
# Clone the repository
git clone https://github.com/shditz/DriftFS.git
cd DriftFS

# Verify code formatting and linting
cargo fmt --all -- --check
cargo clippy --all-targets

# Build release binaries
cargo build --release -p driftfs-ui

# Binary location
.\target\release\driftfs-ui.exe
```

---

## Linux Installation (Scaffolding)

> **Platform Status:** Early Scaffolding. The workspace contains the Linux crate structure and mount configuration models (`driftfs-platform-linux`). Full FUSE 3 userspace callback bindings are in active development.

### Building on Linux

Prerequisites:
- [Rust toolchain](https://rustup.rs/) (1.75+)
- GCC or Clang C++ toolchain
- `libfuse3-dev` (Ubuntu/Debian) or `fuse3-devel` (Fedora/RHEL/Arch)

```bash
# Install system packages (Debian/Ubuntu)
sudo apt-get update
sudo apt-get install -y build-essential libfuse3-dev pkg-config

# Build Linux platform crate and core modules
cargo check -p driftfs-platform-linux
cargo test -p driftfs-platform-linux
```

---

## macOS Installation (Scaffolding)

> **Platform Status:** Early Scaffolding. The workspace contains the macOS crate structure and configuration models (`driftfs-platform-macos`). macFUSE callback integration is in active development.

### Building on macOS

Prerequisites:
- [Rust toolchain](https://rustup.rs/) (1.75+)
- Xcode Command Line Tools (`xcode-select --install`)
- [macFUSE](https://osxfuse.github.io/)

```bash
# Build macOS platform crate and core modules
cargo check -p driftfs-platform-macos
cargo test -p driftfs-platform-macos
```

---

## First-Time Configuration

### 1. Google OAuth 2.0 Credentials

DriftFS uses OAuth 2.0 with PKCE (Proof Key for Code Exchange) to authenticate directly with Google Drive API v3:

* **Official Releases:** Official pre-built releases include a built-in default Client ID. You do not need to configure Google Cloud credentials manually. Simply proceed to step 2.
* **Custom GCP Project (Optional):** If you prefer to use your own Google Cloud project:
  1. Create a Google Cloud project with the **Google Drive API** enabled.
  2. Configure the **OAuth Consent Screen** with Application Type set to External (or Internal), and add the following required scopes:
     - `https://www.googleapis.com/auth/drive`
     - `https://www.googleapis.com/auth/userinfo.email`
     - `https://www.googleapis.com/auth/userinfo.profile`
  3. Under **Credentials**, create an **OAuth 2.0 Client ID** with Application Type set to **Desktop App**.
  4. Obtain your `client_id` (and optional `client_secret`).
  5. Add credentials in the DriftFS Settings tab or add them to your `config.toml`:
     - **Windows:** `%APPDATA%\DriftFS\config.toml`
     - **Linux:** `~/.config/DriftFS/config.toml`
     - **macOS:** `~/Library/Application Support/DriftFS/config.toml`

### 2. Authenticating

1. Start `driftfs-ui`.
2. Click **Connect Google Drive** (or open Settings to enter custom credentials).
3. DriftFS opens your default web browser to the Google OAuth consent screen.
4. Authorize the application. The local loopback listener captures the authorization code and securely commits tokens to your native OS keyring (Windows Credential Manager, macOS Keychain, or Linux Secret Service).
5. Tokens are never saved to plaintext configuration files or logs.

### 3. Mounting the Virtual Drive

1. In the DriftFS dashboard, click **Connect (G:)**.
2. Open Windows Explorer (`Win + E`). Drive `G:` appears under **This PC**.
3. Directories and files stream on demand without consuming local disk space until opened.

---

## Verifying Installation Integrity

Each release provides an authoritative `SHA256SUMS.txt` manifest. Verify your download before execution:

### Windows PowerShell

```powershell
Get-FileHash .\driftfs-setup-v0.1.0-windows-x86_64.exe -Algorithm SHA256
```

Compare the resulting hash against `SHA256SUMS.txt`.

### Linux & macOS

```bash
sha256sum -c SHA256SUMS.txt --ignore-missing
```

---

## Troubleshooting

- **"WinFsp runtime not found"**: Verify that WinFsp is installed and the `Launcher` service is running in `services.msc`.
- **"Drive letter G: already in use"**: Open `%APPDATA%\DriftFS\config.toml` and change `mount_point = "X:"` to an unused drive letter.
- **OAuth loopback redirection fails**: DriftFS binds an ephemeral OS-assigned port (`127.0.0.1:0`). Ensure your local firewall allows loopback connections on `127.0.0.1` and corporate proxies do not intercept localhost traffic.
- **Keyring access denied**: Ensure the user account has access to the local Windows Credential Manager.
- **Metadata location**: Local metadata and staging files reside in `%LOCALAPPDATA%\DriftFS\metadata.db` and `%LOCALAPPDATA%\DriftFS\staging`.

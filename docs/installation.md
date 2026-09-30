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

1. Download the latest installer `driftfs-setup-v0.1.0-windows-x86_64.exe` from the [GitHub Releases](https://github.com/shditz/DriftFS/releases).
2. Run the installer. The setup wizard automatically checks for the WinFsp runtime:
   - If WinFsp is missing, the installer prompts to open the official WinFsp download page.
   - Install WinFsp, then complete the DriftFS setup wizard.
3. Launch **DriftFS** from the Start Menu or Desktop shortcut.

### Option 2: Portable Zip Archive

1. Download `driftfs-v0.1.0-windows-x86_64.zip` from [GitHub Releases](https://github.com/shditz/DriftFS/releases).
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

## First-Time Configuration

### 1. Google OAuth 2.0 Credentials

DriftFS uses OAuth 2.0 with PKCE (Proof Key for Code Exchange) to authenticate directly with Google Drive API v3:

1. Create a Google Cloud project with the **Google Drive API** enabled.
2. Under **Credentials**, create an **OAuth 2.0 Client ID** with Application Type set to **Desktop App**.
3. Obtain your `client_id` and `client_secret`.
4. Copy `config.example.toml` to your configuration directory:
   - **Windows:** `%APPDATA%\DriftFS\config.toml`
   - **Linux:** `~/.config/DriftFS/config.toml`
   - **macOS:** `~/Library/Application Support/DriftFS/config.toml`

### 2. Authenticating

1. Start `driftfs-ui`.
2. Click **Connect Google Account**.
3. DriftFS opens your default web browser to the Google OAuth consent screen.
4. Authorize the application. The local loopback listener captures the authorization token and securely commits the refresh token to your OS keyring (Windows Credential Manager).
5. Tokens are never saved to plaintext configuration files or logs.

### 3. Mounting the Virtual Drive

1. In the DriftFS dashboard, click **Mount Drive (G:)**.
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
- **Port 8080 collision during OAuth**: If another application is listening on the default loopback port, terminate that process or restart DriftFS to allocate an alternate loopback port.
- **Keyring access denied**: Ensure the user account has access to the local Windows Credential Manager.

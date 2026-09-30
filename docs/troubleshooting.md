# Troubleshooting

Common issues and their solutions when running DriftFS on Windows.

---

## WinFsp Driver Not Found

**Symptom:** Application fails to start with an error about WinFsp DLL or driver not being found.

**Cause:** WinFsp is not installed or was installed without the Developer runtime.

**Fix:**
1. Download WinFsp from [winfsp.dev](https://winfsp.dev/).
2. During installation, check the **Developer** feature checkbox.
3. Restart your terminal and rebuild: `cargo build --all-targets`.

**Verification:** Run `where winfsp-x64.dll`: if it returns a path, WinFsp is correctly installed.

---

## Drive Letter Already In Use

**Symptom:** Mount fails with an error indicating the drive letter is occupied.

**Cause:** The configured mount point (default `G:`) is already assigned to a USB drive, network share, or other storage device.

**Fix:** Change the `mount_point` value in your `config.toml`:

```toml
[mount]
mount_point = "X:"   # Use any available letter
```

**Verification:** Open File Explorer and confirm the chosen letter is not already assigned.

---

## OAuth Login Fails / Browser Does Not Open

**Symptom:** Running the application does not open the browser for Google sign-in, or the browser opens but the redirect fails.

**Cause:** The loopback TCP listener on `127.0.0.1` could not bind a port, or a local firewall is blocking the loopback connection.

**Fix:**
1. Check if another application is using the port. DriftFS binds an ephemeral port, so conflicts are rare.
2. Ensure your firewall allows loopback connections (`127.0.0.1`).
3. If running in a corporate environment with proxy restrictions, ensure `localhost` / `127.0.0.1` traffic is not routed through the proxy.

---

## OS Keyring Access Denied

**Symptom:** Application fails to save or retrieve OAuth tokens with a keyring error.

**Cause:** Windows Credential Manager is unavailable, typically in headless environments, remote desktop sessions without credential delegation, or when running as a different user.

**Fix:**
1. Ensure you are running DriftFS in an interactive desktop session (not as a Windows Service or in a headless CI runner).
2. If running via Remote Desktop, enable "Allow delegating saved credentials" in Group Policy.
3. Verify Windows Credential Manager is accessible: open Start → search "Credential Manager" and confirm it opens.

---

## SQLite Database Locked

**Symptom:** Application logs show "database is locked" errors.

**Cause:** Another DriftFS process or tool has an open connection to the same metadata database file.

**Fix:**
1. Ensure only one DriftFS instance is running at a time.
2. Close any SQLite browser tools that may have the database open.
3. DriftFS uses `busy_timeout = 5000` (5 seconds). Brief lock contention resolves automatically; persistent locking indicates a competing process.

---

## Cache Directory Permissions

**Symptom:** Cache read/write errors in logs.

**Cause:** The configured cache directory does not exist or the current user lacks write permissions.

**Fix:** Ensure the cache directory is writable:

```toml
[cache]
directory = "cache"           # Relative to %APPDATA%\DriftFS
max_size_bytes = 536870912    # 512 MB
```

DriftFS creates the cache directory automatically on first run. If it fails, create it manually and verify permissions.

---

## Google API Rate Limiting

**Symptom:** Requests fail with HTTP 403 or 429 errors in logs.

**Cause:** Google Drive API quota exceeded for your OAuth client.

**Fix:**
1. DriftFS uses exponential backoff with jitter for transient errors. Most rate limiting resolves automatically.
2. If persistent, reduce `network.max_concurrent_requests` in `config.toml`.
3. Check your Google Cloud Console for API quota usage and increase limits if needed.

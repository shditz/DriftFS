# `driftfs-ui`

Desktop GUI dashboard and Windows system tray controller for DriftFS built with Slint and `tray-icon`.

## Scope

- **Slint Desktop GUI**: Minimalist, responsive native interface displaying mount status, drive letter selection, quota gauge, and account status.
- **System Tray Integration**: Background tray icon with context menu actions (Show Window, Mount/Unmount, Settings, Quit).
- **Single Instance Enforcement**: Uses a named Win32 mutex (`Local\DriftFS_SingleInstance_Mutex`) on Windows to prevent duplicate running instances.
- **Lifecycle Coordination**: Orchestrates OAuth authentication, WinFsp virtual filesystem mounting, background sync worker tasks, and clean unmounting on application shutdown.
- **Asynchronous Controller**: Manages inter-thread messaging between the Slint UI thread, Tokio async runtime, and WinFsp filesystem threads.

## Architecture

- `src/main.rs`: Entry point, Slint UI component instantiation, single instance guard, and event loop wiring.
- `src/controller.rs`: `AppController` coordinating OAuth, config persistence, WinFsp mounting, and background sync.
- `src/tray.rs`: `SystemTrayManager` wrapping `tray-icon` for tray icon updates and context menu events.
- `ui/`: Slint declarative UI markup (`app.slint`, `components.slint`, `theme.slint`).

## Running Locally

```bash
cargo run -p driftfs-ui
```

To run with verbose debug logs:

```bash
cargo run -p driftfs-ui -- --log-level debug
```

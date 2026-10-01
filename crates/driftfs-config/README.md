# `driftfs-config`

Configuration schema parsing, validation, serialization, and default operating system path resolution for DriftFS.

## Scope

- **TOML Configuration**: Defines schema sections (`[auth]`, `[mount]`, `[cache]`, `[network]`, `[sync]`, `[logging]`, `[gui]`).
- **OS Path Resolution**: Computes default configuration and cache directories across Windows, Linux, and macOS using `dirs`.
- **Graceful Fallbacks**: Returns validated defaults when configuration files are absent.

## Primary Exports

- `DriftFsConfig`: Root configuration struct.
- `AuthConfig`, `MountConfig`, `CacheConfig`, `NetworkConfig`, `SyncConfig`, `LoggingConfig`, `GuiConfig`: Section structures.
- `DriftFsConfig::load(path)`: Reads and deserializes TOML configuration.
- `DriftFsConfig::save(path)`: Serializes and writes configuration atomically.
- `DriftFsConfig::default_path()`: Returns `%APPDATA%\DriftFS\config.toml` (Windows) or `~/.config/DriftFS/config.toml` (Unix).

## Example

```rust
use driftfs_config::DriftFsConfig;

let config_path = DriftFsConfig::default_path();
let config = DriftFsConfig::load(&config_path).unwrap_or_default();

println!("Mount point: {}", config.mount.mount_point);
println!("Cache limit: {} MB", config.cache.max_size_bytes / (1024 * 1024));
```

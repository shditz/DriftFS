use driftfs_core::DriftFsError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftFsConfig {
    pub version: u32,
    pub mount: MountConfig,
    pub cache: CacheConfig,
    pub network: NetworkConfig,
    pub sync: SyncConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    pub mount_point: String,
    pub auto_mount: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub directory: PathBuf,
    pub max_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub max_concurrent_requests: usize,
    pub prefetch_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for DriftFsConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            mount: MountConfig {
                mount_point: default_mount_point(),
                auto_mount: false,
            },
            cache: CacheConfig {
                directory: default_cache_dir(),
                max_size_bytes: 512 * 1024 * 1024,
            },
            network: NetworkConfig {
                max_concurrent_requests: 4,
                prefetch_enabled: true,
            },
            sync: SyncConfig {
                poll_interval_secs: 60,
            },
            logging: LoggingConfig {
                level: "info".into(),
            },
        }
    }
}

impl DriftFsConfig {
    pub fn load(path: &Path) -> driftfs_core::Result<Self> {
        if !path.exists() {
            tracing::info!(?path, "config file not found, using defaults");
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path).map_err(|e| DriftFsError::Configuration {
            message: format!("failed to read config at {}: {e}", path.display()),
        })?;

        let config: Self = toml::from_str(&content).map_err(|e| DriftFsError::Configuration {
            message: format!("failed to parse config: {e}"),
        })?;

        if config.version > CONFIG_VERSION {
            return Err(DriftFsError::Configuration {
                message: format!(
                    "config version {} is newer than supported version {CONFIG_VERSION}",
                    config.version
                ),
            });
        }

        Ok(config)
    }

    pub fn save(&self, path: &Path) -> driftfs_core::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DriftFsError::Configuration {
                message: format!("failed to create config directory: {e}"),
            })?;
        }

        let content = toml::to_string_pretty(self).map_err(|e| DriftFsError::Serialization {
            message: format!("failed to serialize config: {e}"),
            source: Some(Box::new(e)),
        })?;

        std::fs::write(path, content).map_err(|e| DriftFsError::Configuration {
            message: format!("failed to write config to {}: {e}", path.display()),
        })
    }

    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("DriftFS")
            .join("config.toml")
    }
}

fn default_mount_point() -> String {
    if cfg!(windows) {
        "G:".into()
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("DriftFS")
            .join("GoogleDrive")
            .to_string_lossy()
            .into_owned()
    }
}

fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("driftfs")
        .join("cache")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips_through_toml() {
        let config = DriftFsConfig::default();
        let serialized = toml::to_string_pretty(&config).expect("serialize");
        let deserialized: DriftFsConfig = toml::from_str(&serialized).expect("deserialize");

        assert_eq!(deserialized.version, CONFIG_VERSION);
        assert_eq!(
            deserialized.cache.max_size_bytes,
            config.cache.max_size_bytes
        );
        assert_eq!(
            deserialized.network.max_concurrent_requests,
            config.network.max_concurrent_requests
        );
    }

    #[test]
    fn rejects_future_version() {
        let toml_str = r#"
version = 999
[mount]
mount_point = "G:"
auto_mount = false
[cache]
directory = "."
max_size_bytes = 100
[network]
max_concurrent_requests = 2
prefetch_enabled = false
[sync]
poll_interval_secs = 30
[logging]
level = "debug"
"#;
        let tmp = std::env::temp_dir().join("driftfs_test_config_future.toml");
        std::fs::write(&tmp, toml_str).unwrap();
        let result = DriftFsConfig::load(&tmp);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let result = DriftFsConfig::load(Path::new("/nonexistent/config.toml"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().version, CONFIG_VERSION);
    }
}

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MacosPlatformError {
    #[error("macfuse / fileprovider error: {0}")]
    MountError(String),
    #[error("mount path not found: {0}")]
    InvalidMountPoint(PathBuf),
    #[error("unsupported platform: macos platform adapter requires target_os = \"macos\"")]
    UnsupportedPlatform,
}

#[derive(Debug, Clone)]
pub struct MacosMountConfig {
    pub mount_point: PathBuf,
    pub volume_name: String,
    pub read_only: bool,
}

impl MacosMountConfig {
    pub fn new(mount_point: impl Into<PathBuf>, volume_name: impl Into<String>) -> Self {
        Self {
            mount_point: mount_point.into(),
            volume_name: volume_name.into(),
            read_only: false,
        }
    }
}

pub struct MacosMount {
    config: MacosMountConfig,
    mounted: bool,
}

impl MacosMount {
    pub fn new(config: MacosMountConfig) -> Self {
        Self {
            config,
            mounted: false,
        }
    }

    pub fn mount_point(&self) -> &std::path::Path {
        &self.config.mount_point
    }

    pub fn volume_name(&self) -> &str {
        &self.config.volume_name
    }

    pub fn is_mounted(&self) -> bool {
        self.mounted
    }

    #[cfg(target_os = "macos")]
    pub async fn mount(&mut self) -> std::result::Result<(), MacosPlatformError> {
        Err(MacosPlatformError::MountError(
            "macOS macFUSE mount adapter not yet implemented; platform skeleton only".into(),
        ))
    }

    #[cfg(not(target_os = "macos"))]
    pub async fn mount(&mut self) -> std::result::Result<(), MacosPlatformError> {
        Err(MacosPlatformError::UnsupportedPlatform)
    }

    pub fn unmount(&mut self) -> std::result::Result<(), MacosPlatformError> {
        self.mounted = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_mount_config_defaults() {
        let config = MacosMountConfig::new("/Volumes/DriftFS", "DriftFS");
        assert_eq!(config.mount_point, PathBuf::from("/Volumes/DriftFS"));
        assert_eq!(config.volume_name, "DriftFS");
        assert!(!config.read_only);

        let mount = MacosMount::new(config);
        assert!(!mount.is_mounted());
    }
}

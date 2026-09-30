use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LinuxPlatformError {
    #[error("fuse error: {0}")]
    Fuse(String),
    #[error("mount path not found: {0}")]
    InvalidMountPoint(PathBuf),
    #[error("unsupported platform: linux platform adapter requires target_os = \"linux\"")]
    UnsupportedPlatform,
}

#[derive(Debug, Clone)]
pub struct LinuxMountConfig {
    pub mount_point: PathBuf,
    pub read_only: bool,
    pub allow_other: bool,
}

impl LinuxMountConfig {
    pub fn new(mount_point: impl Into<PathBuf>) -> Self {
        Self {
            mount_point: mount_point.into(),
            read_only: false,
            allow_other: false,
        }
    }
}

pub struct LinuxMount {
    config: LinuxMountConfig,
    mounted: bool,
}

impl LinuxMount {
    pub fn new(config: LinuxMountConfig) -> Self {
        Self {
            config,
            mounted: false,
        }
    }

    pub fn mount_point(&self) -> &std::path::Path {
        &self.config.mount_point
    }

    pub fn is_mounted(&self) -> bool {
        self.mounted
    }

    #[cfg(target_os = "linux")]
    pub async fn mount(&mut self) -> std::result::Result<(), LinuxPlatformError> {
        self.mounted = true;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn mount(&mut self) -> std::result::Result<(), LinuxPlatformError> {
        Err(LinuxPlatformError::UnsupportedPlatform)
    }

    pub fn unmount(&mut self) -> std::result::Result<(), LinuxPlatformError> {
        self.mounted = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_mount_config_defaults() {
        let config = LinuxMountConfig::new("/mnt/driftfs");
        assert_eq!(config.mount_point, PathBuf::from("/mnt/driftfs"));
        assert!(!config.read_only);
        assert!(!config.allow_other);

        let mount = LinuxMount::new(config);
        assert!(!mount.is_mounted());
    }
}

#[cfg(windows)]
pub use driftfs_platform_windows::{
    find_available_drive_letter, DriftFsMount as PlatformMount, MountConfig,
};

#[cfg(not(windows))]
use driftfs_core::{DriftFsError, Result};
#[cfg(not(windows))]
use driftfs_filesystem::DriftFsVfs;
#[cfg(not(windows))]
use driftfs_google_drive::GoogleDriveProvider;
#[cfg(not(windows))]
use std::sync::Arc;

#[cfg(not(windows))]
pub struct PlatformMount;

#[cfg(not(windows))]
#[derive(Debug, Clone)]
pub struct MountConfig {
    pub drive_letter: Option<char>,
    pub volume_label: String,
}

#[cfg(not(windows))]
pub fn find_available_drive_letter(_preferred: Option<char>) -> Result<char> {
    Ok('/')
}

#[cfg(not(windows))]
impl PlatformMount {
    pub fn mount(
        _vfs: Arc<DriftFsVfs<GoogleDriveProvider>>,
        _rt_handle: tokio::runtime::Handle,
        _config: MountConfig,
    ) -> Result<Self> {
        Err(DriftFsError::Filesystem {
            message: "Native mounting is currently supported on Windows with WinFsp".into(),
            source: None,
        })
    }

    pub fn unmount(&mut self) {}
}

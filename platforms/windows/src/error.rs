use driftfs_filesystem::VfsError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("WinFsp initialization failed: {0:?}")]
    InitFailed(winfsp_wrs::InitError),

    #[error("failed to mount WinFsp filesystem with status: {0:#X}")]
    MountFailed(i32),

    #[error("no available drive letters found for mounting")]
    NoDriveLetterAvailable,

    #[error("invalid drive letter requested: '{0}', must be between 'D' and 'Z'")]
    InvalidDriveLetter(char),

    #[error("requested drive letter '{0}:' is already in use")]
    DriveLetterInUse(char),

    #[error("invalid volume label: {0}")]
    InvalidVolumeLabel(String),

    #[error("security descriptor creation failed: {0}")]
    SecurityDescriptor(String),

    #[error("underlying virtual filesystem error: {0}")]
    Vfs(#[from] VfsError),
}

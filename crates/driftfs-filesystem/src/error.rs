use driftfs_core::DriftFsError;
use driftfs_metadata::MetadataError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VfsError {
    #[error("no such file or directory")]
    NotFound,

    #[error("permission denied")]
    AccessDenied,

    #[error("read-only file system")]
    ReadOnly,

    #[error("is a directory")]
    IsDirectory,

    #[error("not a directory")]
    NotDirectory,

    #[error("directory not empty")]
    DirectoryNotEmpty,

    #[error("file already exists")]
    AlreadyExists,

    #[error("invalid file handle")]
    InvalidHandle,

    #[error("invalid path")]
    InvalidPath,

    #[error("rate limited by provider")]
    RateLimited,

    #[error("i/o error: {0}")]
    Io(String),
}

impl VfsError {
    pub fn posix_code(&self) -> i32 {
        match self {
            Self::NotFound => 2,           // ENOENT
            Self::Io(_) => 5,              // EIO
            Self::InvalidHandle => 9,      // EBADF
            Self::AccessDenied => 13,      // EACCES
            Self::RateLimited => 16,       // EBUSY
            Self::AlreadyExists => 17,     // EEXIST
            Self::NotDirectory => 20,      // ENOTDIR
            Self::IsDirectory => 21,       // EISDIR
            Self::InvalidPath => 22,       // EINVAL
            Self::ReadOnly => 30,          // EROFS
            Self::DirectoryNotEmpty => 39, // ENOTEMPTY
        }
    }

    pub fn ntstatus(&self) -> i32 {
        match self {
            Self::NotFound => -1073741772,          // STATUS_OBJECT_NAME_NOT_FOUND
            Self::Io(_) => -1073741643,             // STATUS_UNEXPECTED_IO_ERROR
            Self::InvalidHandle => -1073741816,     // STATUS_INVALID_HANDLE
            Self::AccessDenied => -1073741790,      // STATUS_ACCESS_DENIED
            Self::RateLimited => -1073741789,       // STATUS_DEVICE_BUSY
            Self::AlreadyExists => -1073741771,     // STATUS_OBJECT_NAME_COLLISION (0xC0000035)
            Self::NotDirectory => -1073741769,      // STATUS_NOT_A_DIRECTORY (0xC0000037)
            Self::IsDirectory => -1073741638,       // STATUS_FILE_IS_A_DIRECTORY (0xC00000BA)
            Self::InvalidPath => -1073741773,       // STATUS_OBJECT_PATH_INVALID (0xC0000033)
            Self::ReadOnly => -1073741784,          // STATUS_MEDIA_WRITE_PROTECTED (0xC00000A8)
            Self::DirectoryNotEmpty => -1073741567, // STATUS_DIRECTORY_NOT_EMPTY (0xC0000101)
        }
    }
}

impl From<DriftFsError> for VfsError {
    fn from(err: DriftFsError) -> Self {
        match err {
            DriftFsError::NotFound { .. } => Self::NotFound,
            DriftFsError::Authentication { .. } => Self::AccessDenied,
            DriftFsError::Authorization { .. } => Self::AccessDenied,
            DriftFsError::RateLimited { .. } => Self::RateLimited,
            DriftFsError::Filesystem { message, .. } => Self::Io(message),
            DriftFsError::Network { message, .. } => Self::Io(message),
            DriftFsError::Storage { message, .. } => Self::Io(message),
            DriftFsError::Cache { message } => Self::Io(message),
            DriftFsError::Serialization { message, .. } => Self::Io(message),
            DriftFsError::SyncCheckpointExpired { message } => Self::Io(message),
            DriftFsError::Internal { message } => Self::Io(message),
            DriftFsError::Provider { message, .. } => Self::Io(message),
            DriftFsError::Conflict { message } => Self::Io(message),
            DriftFsError::Configuration { message } => Self::Io(message),
        }
    }
}

impl From<MetadataError> for VfsError {
    fn from(err: MetadataError) -> Self {
        match err {
            MetadataError::NotFound(_) => Self::NotFound,
            MetadataError::Sqlite(e) => Self::Io(e.to_string()),
            MetadataError::MigrationFailed { version, reason } => {
                Self::Io(format!("migration v{version}: {reason}"))
            }
            MetadataError::InvalidState(e) => Self::Io(e),
        }
    }
}

pub type Result<T> = std::result::Result<T, VfsError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vfs_error_ntstatus_mappings() {
        assert_eq!(VfsError::NotFound.ntstatus(), -1073741772);
        assert_eq!(VfsError::AlreadyExists.ntstatus(), -1073741771);
        assert_eq!(VfsError::IsDirectory.ntstatus(), -1073741638);
        assert_eq!(VfsError::NotDirectory.ntstatus(), -1073741769);
        assert_eq!(VfsError::AccessDenied.ntstatus(), -1073741790);
        assert_eq!(VfsError::DirectoryNotEmpty.ntstatus(), -1073741567);
    }
}

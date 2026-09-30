use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("provider error: {0}")]
    Provider(#[from] driftfs_core::DriftFsError),

    #[error("metadata error: {0}")]
    Metadata(#[from] driftfs_metadata::MetadataError),

    #[error("checkpoint error: {0}")]
    Checkpoint(String),

    #[error("sync interrupted")]
    Interrupted,
}

pub type Result<T> = std::result::Result<T, SyncError>;

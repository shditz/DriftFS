use driftfs_core::DriftFsError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("migration failed at version {version}: {reason}")]
    MigrationFailed { version: u32, reason: String },

    #[error("object not found: {0}")]
    NotFound(String),

    #[error("invalid state: {0}")]
    InvalidState(String),
}

impl From<MetadataError> for DriftFsError {
    fn from(err: MetadataError) -> Self {
        DriftFsError::Storage {
            message: err.to_string(),
            source: Some(Box::new(err)),
        }
    }
}

pub type Result<T> = std::result::Result<T, MetadataError>;

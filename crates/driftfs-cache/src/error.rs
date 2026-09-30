use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Cache corruption: {0}")]
    Corruption(String),

    #[error("Invalid cache key: {0}")]
    InvalidKey(String),

    #[error("Cache lock poisoned")]
    LockPoisoned,
}

pub type Result<T> = std::result::Result<T, CacheError>;

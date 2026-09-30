use thiserror::Error;

#[derive(Debug, Error)]
pub enum DriftFsError {
    #[error("authentication failed: {message}")]
    Authentication {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("authorization denied: {message}")]
    Authorization { message: String },

    #[error("provider error: {message}")]
    Provider {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("network error: {message}")]
    Network {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("{}", format_rate_limited(*retry_after_secs))]
    RateLimited { retry_after_secs: Option<u64> },

    #[error("not found: {message}")]
    NotFound { message: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error("filesystem error: {message}")]
    Filesystem {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("storage error: {message}")]
    Storage {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("cache error: {message}")]
    Cache { message: String },

    #[error("serialization error: {message}")]
    Serialization {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("sync checkpoint expired: {message}")]
    SyncCheckpointExpired { message: String },

    #[error("configuration error: {message}")]
    Configuration { message: String },

    #[error("internal error: {message}")]
    Internal { message: String },
}

impl DriftFsError {
    pub fn auth(message: impl Into<String>) -> Self {
        Self::Authentication {
            message: message.into(),
            source: None,
        }
    }

    pub fn auth_with_source(
        message: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::Authentication {
            message: message.into(),
            source: Some(source),
        }
    }

    pub fn network_with_source(
        message: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::Network {
            message: message.into(),
            source: Some(source),
        }
    }

    pub fn storage_with_source(
        message: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::Storage {
            message: message.into(),
            source: Some(source),
        }
    }

    pub fn serialization_with_source(
        message: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::Serialization {
            message: message.into(),
            source: Some(source),
        }
    }

    pub fn sync_checkpoint_expired(message: impl Into<String>) -> Self {
        Self::SyncCheckpointExpired {
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

fn format_rate_limited(retry_after_secs: Option<u64>) -> String {
    match retry_after_secs {
        Some(secs) => format!("rate limited: retry after {secs}s"),
        None => "rate limited: retry later".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_is_meaningful() {
        let err = DriftFsError::NotFound {
            message: "file abc123 does not exist".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("abc123"), "display must include context");
    }

    #[test]
    fn rate_limited_shows_retry() {
        let err = DriftFsError::RateLimited {
            retry_after_secs: Some(30),
        };
        assert_eq!(err.to_string(), "rate limited: retry after 30s");

        let err_none = DriftFsError::RateLimited {
            retry_after_secs: None,
        };
        assert_eq!(err_none.to_string(), "rate limited: retry later");
    }

    #[test]
    fn error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DriftFsError>();
    }
}

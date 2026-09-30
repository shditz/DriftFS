use tracing_subscriber::{fmt, EnvFilter};

/// `DRIFTFS_LOG` environment variable overrides the default filter directive.
pub fn init(filter_directive: &str) {
    let env_filter =
        EnvFilter::try_from_env("DRIFTFS_LOG").unwrap_or_else(|_| EnvFilter::new(filter_directive));

    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
}

pub fn redact(value: &str) -> String {
    if cfg!(debug_assertions) && std::env::var("DRIFTFS_LOG_SECRETS").is_ok() {
        let preview: String = value.chars().take(4).collect();
        format!("{preview}…[REDACTED]")
    } else {
        "[REDACTED]".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_does_not_panic_on_repeated_calls() {
        init("info");
        init("debug");
    }

    #[test]
    fn redact_hides_value_in_release_mode() {
        std::env::remove_var("DRIFTFS_LOG_SECRETS");
        let result = redact("ya29.super-secret-token-value");
        assert_eq!(result, "[REDACTED]");
        assert!(!result.contains("ya29"));
    }
}

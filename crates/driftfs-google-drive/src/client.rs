use driftfs_auth::TokenProvider;
use driftfs_core::{DriftFsError, Result};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DriveClientConfig {
    pub base_url: String,
    pub max_retries: usize,
    pub initial_backoff: Duration,
    pub request_timeout: Duration,
}

impl Default for DriveClientConfig {
    fn default() -> Self {
        Self {
            base_url: "https://www.googleapis.com/drive/v3".into(),
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            request_timeout: Duration::from_secs(30),
        }
    }
}

pub struct GoogleDriveHttpClient {
    token_provider: Arc<dyn TokenProvider>,
    http: Client,
    config: DriveClientConfig,
}

impl GoogleDriveHttpClient {
    pub fn new(token_provider: Arc<dyn TokenProvider>, config: DriveClientConfig) -> Self {
        let http = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .unwrap_or_default();

        Self {
            token_provider,
            http,
            config,
        }
    }

    pub fn token_provider(&self) -> &Arc<dyn TokenProvider> {
        &self.token_provider
    }

    pub async fn execute_request<F>(&self, build_req: F) -> Result<Response>
    where
        F: Fn(&Client, &str, &str) -> RequestBuilder,
    {
        let mut attempts = 0;
        let mut backoff = self.config.initial_backoff;
        let mut refreshed_on_401 = false;

        loop {
            attempts += 1;
            let token = self.token_provider.get_access_token().await?;

            let builder = build_req(&self.http, &self.config.base_url, &token);
            let resp = match builder.send().await {
                Ok(resp) => resp,
                Err(e) if attempts <= self.config.max_retries => {
                    tracing::warn!(attempt = attempts, error = %e, "network error, retrying");
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                    continue;
                }
                Err(e) => {
                    return Err(DriftFsError::network_with_source(
                        format!("Google Drive request failed after {attempts} attempts: {e}"),
                        Box::new(e),
                    ));
                }
            };

            let status = resp.status();

            if status.is_success()
                || status == StatusCode::PARTIAL_CONTENT
                || status == StatusCode::PERMANENT_REDIRECT
            {
                return Ok(resp);
            }

            if status == StatusCode::UNAUTHORIZED && !refreshed_on_401 {
                tracing::info!(
                    "received 401 Unauthorized from Google Drive, forcing token refresh"
                );
                if self.token_provider.force_refresh().await.is_ok() {
                    refreshed_on_401 = true;
                    continue;
                }
            }

            if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                && attempts <= self.config.max_retries
            {
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(Duration::from_secs);

                let wait_duration = retry_after.unwrap_or(backoff);
                tracing::warn!(
                    status = %status,
                    attempt = attempts,
                    wait_ms = wait_duration.as_millis(),
                    "rate limited or transient server error, backing off"
                );
                tokio::time::sleep(wait_duration).await;
                backoff *= 2;
                continue;
            }

            return Err(Self::translate_error(resp).await);
        }
    }

    pub async fn get_json<T, F>(&self, build_req: F) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn(&Client, &str, &str) -> RequestBuilder,
    {
        let resp = self.execute_request(build_req).await?;
        resp.json().await.map_err(|e| {
            DriftFsError::serialization_with_source(
                format!("failed to deserialize Google Drive response: {e}"),
                Box::new(e),
            )
        })
    }

    async fn translate_error(resp: Response) -> DriftFsError {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        match status {
            StatusCode::UNAUTHORIZED => {
                DriftFsError::auth(format!("Google Drive returned 401 Unauthorized: {body}"))
            }
            StatusCode::NOT_FOUND => {
                DriftFsError::not_found(format!("Google Drive file or resource not found: {body}"))
            }
            StatusCode::GONE => {
                DriftFsError::sync_checkpoint_expired(format!("Google Drive 410 Gone: {body}"))
            }
            StatusCode::TOO_MANY_REQUESTS => DriftFsError::RateLimited {
                retry_after_secs: None,
            },
            StatusCode::FORBIDDEN => {
                if body.contains("userRateLimitExceeded") || body.contains("rateLimitExceeded") {
                    DriftFsError::RateLimited {
                        retry_after_secs: None,
                    }
                } else if body.contains("storageQuotaExceeded") {
                    DriftFsError::Storage {
                        message: "Google Drive storage quota exceeded".into(),
                        source: None,
                    }
                } else {
                    DriftFsError::Authorization {
                        message: format!("Google Drive 403 Forbidden: {body}"),
                    }
                }
            }
            StatusCode::CONFLICT => DriftFsError::Conflict {
                message: format!("Google Drive 409 Conflict: {body}"),
            },
            _ => {
                if body.contains("startPageToken") || body.contains("Invalid token") {
                    DriftFsError::sync_checkpoint_expired(format!(
                        "Google Drive HTTP {status}: {body}"
                    ))
                } else {
                    DriftFsError::Provider {
                        message: format!("Google Drive HTTP {status}: {body}"),
                        source: None,
                    }
                }
            }
        }
    }
}

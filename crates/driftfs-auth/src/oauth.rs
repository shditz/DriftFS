use crate::types::{OAuthConfig, PkceChallenge, SecretToken};
use driftfs_core::{DriftFsError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleUserInfo {
    pub sub: String,
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: Option<u64>,
    scope: Option<String>,
}

#[derive(Clone)]
pub struct GoogleOAuthClient {
    config: OAuthConfig,
    http: Client,
}

impl GoogleOAuthClient {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }

    pub fn build_auth_url(
        &self,
        pkce: &PkceChallenge,
        state: &str,
        redirect_uri: &str,
    ) -> Result<String> {
        let mut url =
            Url::parse(&self.config.auth_url).map_err(|e| DriftFsError::Configuration {
                message: format!("invalid auth_url: {e}"),
            })?;

        let scopes = self.config.scopes.join(" ");

        url.query_pairs_mut()
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &scopes)
            .append_pair("code_challenge", &pkce.code_challenge)
            .append_pair("code_challenge_method", pkce.method)
            .append_pair("state", state)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");

        Ok(url.to_string())
    }

    pub async fn bind_loopback_listener() -> Result<TcpListener> {
        TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| DriftFsError::Network {
                message: format!("failed to bind loopback TCP listener for OAuth: {e}"),
                source: Some(Box::new(e)),
            })
    }

    pub fn loopback_redirect_uri(listener: &TcpListener) -> Result<String> {
        let addr = listener.local_addr().map_err(|e| DriftFsError::Network {
            message: format!("failed to get listener address: {e}"),
            source: Some(Box::new(e)),
        })?;
        Ok(format!("http://127.0.0.1:{}/callback", addr.port()))
    }

    pub async fn listen_for_callback(
        listener: TcpListener,
        expected_state: &str,
        timeout_dur: Duration,
    ) -> Result<String> {
        tokio::select! {
            result = Self::accept_and_handle_callback(listener, expected_state) => result,
            _ = tokio::time::sleep(timeout_dur) => Err(DriftFsError::auth(
                "timed out waiting for OAuth authorization in browser",
            )),
        }
    }

    async fn accept_and_handle_callback(
        listener: TcpListener,
        expected_state: &str,
    ) -> Result<String> {
        loop {
            let (mut socket, _) = listener.accept().await.map_err(|e| DriftFsError::Network {
                message: format!("failed to accept OAuth callback connection: {e}"),
                source: Some(Box::new(e)),
            })?;

            let mut buf = [0u8; 4096];
            let n = match socket.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => continue,
            };

            let request = String::from_utf8_lossy(&buf[..n]);
            let first_line = request.lines().next().unwrap_or_default();
            let target = first_line.split_whitespace().nth(1).unwrap_or("/");

            if !target.starts_with("/callback") {
                let not_found =
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(not_found.as_bytes()).await;
                continue;
            }

            let dummy_url = format!("http://127.0.0.1{target}");
            let parsed_url = match Url::parse(&dummy_url) {
                Ok(url) => url,
                Err(_) => {
                    let bad_req = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(bad_req.as_bytes()).await;
                    continue;
                }
            };

            let params: HashMap<String, String> = parsed_url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            if let Some(err) = params.get("error") {
                let raw_desc = params
                    .get("error_description")
                    .cloned()
                    .unwrap_or_else(|| err.clone());
                let error_desc = escape_html(&raw_desc);
                let html = format!(
                    "<!DOCTYPE html><html><head><meta charset='utf-8'><title>DriftFS Authentication</title></head>\
                     <body style='margin:0;padding:60px 20px;background:#050505;font-family:-apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,sans-serif;color:#f8fafc;-webkit-font-smoothing:antialiased;'>\
                     <div style='max-width:440px;margin:0 auto;background:rgba(255,255,255,0.03);border:1px solid rgba(255,255,255,0.08);border-radius:24px;padding:6px;box-shadow:0 24px 48px rgba(0,0,0,0.6);'>\
                     <div style='background:#0b0f19;border-radius:18px;padding:36px 28px;box-shadow:inset 0 1px 1px rgba(255,255,255,0.08);text-align:center;'>\
                     <div style='width:44px;height:44px;border-radius:50%;background:rgba(248,113,113,0.1);border:1px solid rgba(248,113,113,0.25);display:flex;align-items:center;justify-content:center;margin:0 auto 16px;'>\
                     <svg aria-hidden='true' width='20' height='20' viewBox='0 0 24 24' fill='none' stroke='#f87171' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><circle cx='12' cy='12' r='10'></circle><line x1='15' y1='9' x2='9' y2='15'></line><line x1='9' y1='9' x2='15' y2='15'></line></svg>\
                     </div>\
                     <h2 style='margin:0 0 8px;font-size:19px;font-weight:600;letter-spacing:-0.01em;color:#f8fafc;'>Sign-in failed</h2>\
                     <p style='margin:0;font-size:14px;color:#94a3b8;line-height:1.5;'>{error_desc}</p>\
                     </div></div></body></html>"
                );
                let response = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline';\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                    html.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                return Err(DriftFsError::auth(format!(
                    "OAuth error from provider: {raw_desc}"
                )));
            }

            let state = match params.get("state") {
                Some(state) => state,
                None => {
                    let bad_req = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(bad_req.as_bytes()).await;
                    continue;
                }
            };

            if state != expected_state {
                return Err(DriftFsError::auth(
                    "CSRF state parameter mismatch in OAuth callback",
                ));
            }

            let code = match params.get("code") {
                Some(code) => code.clone(),
                None => {
                    let bad_req = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(bad_req.as_bytes()).await;
                    continue;
                }
            };

            let html = "<!DOCTYPE html><html><head><meta charset='utf-8'><title>DriftFS Authentication</title></head>\
                        <body style='margin:0;padding:60px 20px;background:#050505;font-family:-apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,sans-serif;color:#f8fafc;-webkit-font-smoothing:antialiased;'>\
                        <div style='max-width:440px;margin:0 auto;background:rgba(255,255,255,0.03);border:1px solid rgba(255,255,255,0.08);border-radius:24px;padding:6px;box-shadow:0 24px 48px rgba(0,0,0,0.6);'>\
                        <div style='background:#0b0f19;border-radius:18px;padding:36px 28px;box-shadow:inset 0 1px 1px rgba(255,255,255,0.08);text-align:center;'>\
                        <div style='width:44px;height:44px;border-radius:50%;background:rgba(56,189,248,0.1);border:1px solid rgba(56,189,248,0.25);display:flex;align-items:center;justify-content:center;margin:0 auto 16px;'>\
                        <svg aria-hidden='true' width='20' height='20' viewBox='0 0 24 24' fill='none' stroke='#38bdf8' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><path d='M20 6L9 17l-5-5'></path></svg>\
                        </div>\
                        <h2 style='margin:0 0 8px;font-size:19px;font-weight:600;letter-spacing:-0.01em;color:#f8fafc;'>Sign-in successful</h2>\
                        <p style='margin:0;font-size:14px;color:#94a3b8;line-height:1.5;'>You can now close this tab and return to DriftFS.</p>\
                        </div></div></body></html>";

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline';\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;

            return Ok(code);
        }
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<SecretToken> {
        let mut form = HashMap::new();
        form.insert("client_id", self.config.client_id.as_str());
        if let Some(secret) = &self.config.client_secret {
            form.insert("client_secret", secret.as_str());
        }
        form.insert("code", code);
        form.insert("code_verifier", code_verifier);
        form.insert("grant_type", "authorization_code");
        form.insert("redirect_uri", redirect_uri);

        let res = self
            .http
            .post(&self.config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|e| DriftFsError::Network {
                message: format!("token exchange request failed: {e}"),
                source: Some(Box::new(e)),
            })?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(DriftFsError::auth(format!(
                "token exchange returned HTTP {status}: {body}"
            )));
        }

        let token_resp: GoogleTokenResponse =
            res.json().await.map_err(|e| DriftFsError::Serialization {
                message: format!("failed to parse token response: {e}"),
                source: Some(Box::new(e)),
            })?;

        let scopes = token_resp
            .scope
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_else(|| self.config.scopes.clone());

        Ok(SecretToken::new(
            token_resp.access_token,
            token_resp.refresh_token,
            token_resp.token_type,
            token_resp.expires_in,
            scopes,
        ))
    }

    pub async fn refresh_access_token(&self, refresh_token: &str) -> Result<SecretToken> {
        let mut form = HashMap::new();
        form.insert("client_id", self.config.client_id.as_str());
        if let Some(secret) = &self.config.client_secret {
            form.insert("client_secret", secret.as_str());
        }
        form.insert("refresh_token", refresh_token);
        form.insert("grant_type", "refresh_token");

        let res = self
            .http
            .post(&self.config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|e| DriftFsError::Network {
                message: format!("token refresh request failed: {e}"),
                source: Some(Box::new(e)),
            })?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(DriftFsError::auth(format!(
                "token refresh returned HTTP {status}: {body}"
            )));
        }

        let token_resp: GoogleTokenResponse =
            res.json().await.map_err(|e| DriftFsError::Serialization {
                message: format!("failed to parse refresh token response: {e}"),
                source: Some(Box::new(e)),
            })?;

        let scopes = token_resp
            .scope
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_else(|| self.config.scopes.clone());

        // Google does not always return a new refresh token on refresh; retain original if not provided.
        let final_refresh_token = token_resp
            .refresh_token
            .or_else(|| Some(refresh_token.to_string()));

        Ok(SecretToken::new(
            token_resp.access_token,
            final_refresh_token,
            token_resp.token_type,
            token_resp.expires_in,
            scopes,
        ))
    }

    pub async fn fetch_userinfo(&self, access_token: &str) -> Result<GoogleUserInfo> {
        let res = self
            .http
            .get(&self.config.userinfo_url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| DriftFsError::Network {
                message: format!("failed to call userinfo endpoint: {e}"),
                source: Some(Box::new(e)),
            })?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(DriftFsError::auth(format!(
                "userinfo returned HTTP {status}: {body}"
            )));
        }

        res.json().await.map_err(|e| DriftFsError::Serialization {
            message: format!("failed to parse userinfo response: {e}"),
            source: Some(Box::new(e)),
        })
    }
}

fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_listener_generates_valid_redirect_uri() {
        let listener = GoogleOAuthClient::bind_loopback_listener().await.unwrap();
        let uri = GoogleOAuthClient::loopback_redirect_uri(&listener).unwrap();
        assert!(uri.starts_with("http://127.0.0.1:"));
        assert!(uri.ends_with("/callback"));
    }

    #[test]
    fn auth_url_construction() {
        let config = OAuthConfig {
            client_id: "test-client-id.apps.googleusercontent.com".into(),
            client_secret: None,
            ..Default::default()
        };
        let client = GoogleOAuthClient::new(config);
        let pkce = PkceChallenge::generate();
        let state = "secure-random-state-123";
        let redirect_uri = "http://127.0.0.1:8080/callback";

        let url_str = client
            .build_auth_url(&pkce, state, redirect_uri)
            .expect("build auth url");
        let parsed = Url::parse(&url_str).expect("valid url");

        assert_eq!(parsed.host_str(), Some("accounts.google.com"));
        let query: HashMap<_, _> = parsed.query_pairs().collect();

        assert_eq!(
            query.get("client_id").unwrap(),
            "test-client-id.apps.googleusercontent.com"
        );
        assert_eq!(query.get("redirect_uri").unwrap(), redirect_uri);
        assert_eq!(query.get("response_type").unwrap(), "code");
        assert_eq!(query.get("code_challenge_method").unwrap(), "S256");
        assert_eq!(query.get("code_challenge").unwrap(), &pkce.code_challenge);
        assert_eq!(query.get("state").unwrap(), state);
        assert_eq!(query.get("access_type").unwrap(), "offline");
        assert_eq!(query.get("prompt").unwrap(), "consent");
    }

    #[tokio::test]
    async fn callback_listener_handles_incoming_request() {
        let listener = GoogleOAuthClient::bind_loopback_listener().await.unwrap();
        let redirect_uri = GoogleOAuthClient::loopback_redirect_uri(&listener).unwrap();
        let expected_state = "test_state_xyz";

        let listen_handle = tokio::spawn(async move {
            GoogleOAuthClient::listen_for_callback(listener, expected_state, Duration::from_secs(5))
                .await
        });

        let client = Client::new();
        let callback_url = format!("{redirect_uri}?code=test_auth_code_123&state={expected_state}");
        let resp = client.get(&callback_url).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers()
                .get("content-security-policy")
                .and_then(|v| v.to_str().ok()),
            Some("default-src 'none'; style-src 'unsafe-inline';")
        );
        assert_eq!(
            resp.headers()
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );

        let code = listen_handle.await.unwrap().expect("code returned");
        assert_eq!(code, "test_auth_code_123");
    }

    #[tokio::test]
    async fn callback_listener_escapes_html_error_and_sets_csp() {
        let listener = GoogleOAuthClient::bind_loopback_listener().await.unwrap();
        let redirect_uri = GoogleOAuthClient::loopback_redirect_uri(&listener).unwrap();
        let expected_state = "test_state_xyz";

        let listen_handle = tokio::spawn(async move {
            GoogleOAuthClient::listen_for_callback(listener, expected_state, Duration::from_secs(5))
                .await
        });

        let client = Client::new();
        let payload = "<script>alert('xss')</script>";
        let resp = client
            .get(&redirect_uri)
            .query(&[("error", "access_denied"), ("error_description", payload)])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);

        let body = resp.text().await.unwrap();
        assert!(!body.contains("<script>alert('xss')</script>"));
        assert!(body.contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"));

        let res = listen_handle.await.unwrap();
        assert!(res.is_err());
    }

    #[test]
    fn html_escape_handles_dangerous_characters() {
        let dangerous = "<div class=\"test\" id='foo'>&bar</div>";
        let escaped = escape_html(dangerous);
        assert_eq!(
            escaped,
            "&lt;div class=&quot;test&quot; id=&#39;foo&#39;&gt;&amp;bar&lt;/div&gt;"
        );
    }
}

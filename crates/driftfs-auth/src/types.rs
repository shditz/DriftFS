use driftfs_core::AccountId;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::{Duration, SystemTime};

/// Holds OAuth tokens. `Debug` redacts secret values to prevent log leakage.
#[derive(Clone, Serialize, Deserialize)]
pub struct SecretToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_at: Option<SystemTime>,
    pub scopes: Vec<String>,
}

impl SecretToken {
    pub fn new(
        access_token: String,
        refresh_token: Option<String>,
        token_type: String,
        expires_in_secs: Option<u64>,
        scopes: Vec<String>,
    ) -> Self {
        let expires_at = expires_in_secs.map(|s| SystemTime::now() + Duration::from_secs(s));
        Self {
            access_token,
            refresh_token,
            token_type,
            expires_at,
            scopes,
        }
    }

    pub fn is_expired_with_buffer(&self, buffer: Duration) -> bool {
        match self.expires_at {
            Some(expiry) => {
                let now = SystemTime::now();
                match expiry.duration_since(now) {
                    Ok(remaining) => remaining <= buffer,
                    Err(_) => true,
                }
            }
            None => false,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.is_expired_with_buffer(Duration::from_secs(60))
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretToken")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("token_type", &self.token_type)
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .finish()
    }
}

#[derive(Clone)]
pub struct PkceChallenge {
    pub code_verifier: String,
    pub code_challenge: String,
    pub method: &'static str,
}

impl PkceChallenge {
    pub fn generate() -> Self {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";
        let mut rng = rand::thread_rng();

        // RFC 7636 allows 43-128 chars; 64 chars provides 384 bits of entropy.
        let verifier: String = (0..64)
            .map(|_| {
                let idx = rng.gen_range(0..CHARS.len());
                CHARS[idx] as char
            })
            .collect();

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();

        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        let challenge = URL_SAFE_NO_PAD.encode(hash);

        Self {
            code_verifier: verifier,
            code_challenge: challenge,
            method: "S256",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountIdentity {
    pub id: AccountId,
    pub provider: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider_account_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub scopes: Vec<String>,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: None,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            userinfo_url: "https://www.googleapis.com/oauth2/v3/userinfo".into(),
            scopes: vec![
                "https://www.googleapis.com/auth/drive".into(),
                "https://www.googleapis.com/auth/userinfo.email".into(),
                "https://www.googleapis.com/auth/userinfo.profile".into(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_token_redacts_in_debug() {
        let token = SecretToken::new(
            "ya29.secret_access_token_123".into(),
            Some("1//secret_refresh_token_456".into()),
            "Bearer".into(),
            Some(3600),
            vec!["drive".into()],
        );

        let debug_str = format!("{token:?}");
        assert!(!debug_str.contains("ya29"));
        assert!(!debug_str.contains("secret_access_token"));
        assert!(!debug_str.contains("secret_refresh_token"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn pkce_challenge_format() {
        let pkce = PkceChallenge::generate();
        assert_eq!(pkce.code_verifier.len(), 64);
        assert_eq!(pkce.method, "S256");
        assert!(!pkce.code_challenge.contains('='));
        assert!(!pkce.code_challenge.contains('+'));
        assert!(!pkce.code_challenge.contains('/'));
        assert_eq!(pkce.code_challenge.len(), 43);
    }

    #[test]
    fn token_expiry_check() {
        let mut token = SecretToken::new("test".into(), None, "Bearer".into(), Some(30), vec![]);
        assert!(token.is_expired());
        assert!(!token.is_expired_with_buffer(Duration::from_secs(10)));

        token.expires_at = Some(SystemTime::now() - Duration::from_secs(10));
        assert!(token.is_expired());
    }
}

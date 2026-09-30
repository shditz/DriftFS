use crate::account_store::AccountStore;
use crate::credential_store::CredentialStore;
use crate::oauth::GoogleOAuthClient;
use crate::types::{AccountIdentity, PkceChallenge};
use driftfs_core::{AccountId, DriftFsError, Result};
use rand::Rng;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

pub trait TokenProvider: Send + Sync {
    fn get_access_token<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;
    fn force_refresh<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        self.get_access_token()
    }
    fn account_id(&self) -> &AccountId;
}

pub struct LoginSession {
    pub auth_url: String,
    pub redirect_uri: String,
    pub pkce: PkceChallenge,
    pub state: String,
    pub listener: TcpListener,
}

pub struct AuthService {
    oauth: GoogleOAuthClient,
    credentials: Arc<dyn CredentialStore>,
    accounts: Arc<AccountStore>,
}

impl AuthService {
    pub fn new(
        oauth: GoogleOAuthClient,
        credentials: Arc<dyn CredentialStore>,
        accounts: Arc<AccountStore>,
    ) -> Self {
        Self {
            oauth,
            credentials,
            accounts,
        }
    }

    pub fn accounts(&self) -> &Arc<AccountStore> {
        &self.accounts
    }

    pub fn credentials(&self) -> &Arc<dyn CredentialStore> {
        &self.credentials
    }

    pub async fn start_login_flow(&self) -> Result<LoginSession> {
        let listener = GoogleOAuthClient::bind_loopback_listener().await?;
        let redirect_uri = GoogleOAuthClient::loopback_redirect_uri(&listener)?;
        let pkce = PkceChallenge::generate();

        let mut rng = rand::thread_rng();
        let state: String = (0..32)
            .map(|_| rng.gen_range(b'a'..=b'z') as char)
            .collect();

        let auth_url = self.oauth.build_auth_url(&pkce, &state, &redirect_uri)?;

        Ok(LoginSession {
            auth_url,
            redirect_uri,
            pkce,
            state,
            listener,
        })
    }

    pub async fn complete_login_flow(
        &self,
        session: LoginSession,
        timeout_dur: Duration,
    ) -> Result<AccountIdentity> {
        let code =
            GoogleOAuthClient::listen_for_callback(session.listener, &session.state, timeout_dur)
                .await?;

        let token = self
            .oauth
            .exchange_code(&code, &session.pkce.code_verifier, &session.redirect_uri)
            .await?;

        let userinfo = self.oauth.fetch_userinfo(&token.access_token).await?;

        // Prefix prevents ID collisions across multiple identity providers.
        let account_id = AccountId(format!("google:{}", userinfo.sub));

        self.credentials.save_token(&account_id, &token)?;

        let now = unix_epoch_timestamp_string();
        let identity = AccountIdentity {
            id: account_id.clone(),
            provider: "google".into(),
            email: userinfo.email,
            display_name: userinfo.name,
            provider_account_id: userinfo.sub,
            created_at: now,
        };

        self.accounts.save_account(&identity)?;

        tracing::info!(
            account_id = %account_id,
            email = %identity.email,
            "successfully authenticated account"
        );

        Ok(identity)
    }

    pub async fn get_valid_access_token(&self, account_id: &AccountId) -> Result<String> {
        let mut token = self.credentials.load_token(account_id)?.ok_or_else(|| {
            DriftFsError::auth(format!("no credentials found for account {account_id}"))
        })?;

        if token.is_expired() {
            let refresh_token = token.refresh_token.as_ref().ok_or_else(|| {
                DriftFsError::auth(format!(
                    "access token is expired and no refresh token is stored for {account_id}"
                ))
            })?;

            tracing::info!(account_id = %account_id, "refreshing expired OAuth token");
            let refreshed = self.oauth.refresh_access_token(refresh_token).await?;

            self.credentials.save_token(account_id, &refreshed)?;
            token = refreshed;
        }

        Ok(token.access_token)
    }

    pub async fn force_refresh_access_token(&self, account_id: &AccountId) -> Result<String> {
        let token = self.credentials.load_token(account_id)?.ok_or_else(|| {
            DriftFsError::auth(format!("no credentials found for account {account_id}"))
        })?;

        let refresh_token = token.refresh_token.as_ref().ok_or_else(|| {
            DriftFsError::auth(format!(
                "cannot refresh token: no refresh token is stored for {account_id}"
            ))
        })?;

        tracing::info!(account_id = %account_id, "force-refreshing OAuth token");
        let refreshed = self.oauth.refresh_access_token(refresh_token).await?;

        self.credentials.save_token(account_id, &refreshed)?;
        Ok(refreshed.access_token)
    }

    pub fn create_token_provider(
        self: &Arc<Self>,
        account_id: AccountId,
    ) -> Arc<dyn TokenProvider> {
        Arc::new(AccountTokenProvider {
            auth_service: Arc::clone(self),
            account_id,
        })
    }
}

struct AccountTokenProvider {
    auth_service: Arc<AuthService>,
    account_id: AccountId,
}

impl TokenProvider for AccountTokenProvider {
    fn get_access_token<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            self.auth_service
                .get_valid_access_token(&self.account_id)
                .await
        })
    }

    fn force_refresh<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            self.auth_service
                .force_refresh_access_token(&self.account_id)
                .await
        })
    }

    fn account_id(&self) -> &AccountId {
        &self.account_id
    }
}

fn unix_epoch_timestamp_string() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_store::InMemoryCredentialStore;
    use crate::types::{OAuthConfig, SecretToken};

    #[tokio::test]
    async fn token_provider_returns_unexpired_token() {
        let dir = tempfile::tempdir().unwrap();
        let account_store = Arc::new(AccountStore::new(dir.path().join("accounts.json")).unwrap());
        let cred_store = Arc::new(InMemoryCredentialStore::new());
        let oauth = GoogleOAuthClient::new(OAuthConfig::default());

        let auth_service = Arc::new(AuthService::new(oauth, cred_store.clone(), account_store));

        let account_id = AccountId("test_acc".into());
        let token = SecretToken::new(
            "valid_token_xyz".into(),
            Some("refresh_abc".into()),
            "Bearer".into(),
            Some(3600),
            vec![],
        );

        cred_store.save_token(&account_id, &token).unwrap();

        let provider = auth_service.create_token_provider(account_id.clone());
        let retrieved = provider.get_access_token().await.unwrap();
        assert_eq!(retrieved, "valid_token_xyz");
        assert_eq!(provider.account_id(), &account_id);
    }
}

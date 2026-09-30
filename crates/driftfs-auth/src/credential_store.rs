use crate::types::SecretToken;
use driftfs_core::{AccountId, DriftFsError, Result};
use std::collections::HashMap;
use std::sync::RwLock;

pub trait CredentialStore: Send + Sync {
    fn save_token(&self, account_id: &AccountId, token: &SecretToken) -> Result<()>;
    fn load_token(&self, account_id: &AccountId) -> Result<Option<SecretToken>>;
    fn delete_token(&self, account_id: &AccountId) -> Result<()>;
}

pub struct KeyringCredentialStore {
    service_name: String,
}

impl KeyringCredentialStore {
    pub fn new() -> Self {
        Self {
            service_name: "driftfs".into(),
        }
    }

    pub fn with_service_name(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
        }
    }

    fn entry_for(&self, account_id: &AccountId) -> Result<keyring::Entry> {
        let username = format!("account:{}", account_id.0);
        keyring::Entry::new(&self.service_name, &username).map_err(|e| {
            DriftFsError::auth(format!(
                "failed to initialize secure credential store entry: {e}"
            ))
        })
    }
}

impl Default for KeyringCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for KeyringCredentialStore {
    fn save_token(&self, account_id: &AccountId, token: &SecretToken) -> Result<()> {
        let entry = self.entry_for(account_id)?;
        let serialized = serde_json::to_string(token).map_err(|e| DriftFsError::Serialization {
            message: format!("failed to serialize secret token: {e}"),
            source: Some(Box::new(e)),
        })?;

        entry.set_password(&serialized).map_err(|e| {
            DriftFsError::auth(format!("failed to save credentials to OS keyring: {e}"))
        })?;

        tracing::info!(account_id = %account_id, "securely saved credentials to OS keyring");
        Ok(())
    }

    fn load_token(&self, account_id: &AccountId) -> Result<Option<SecretToken>> {
        let entry = self.entry_for(account_id)?;
        match entry.get_password() {
            Ok(secret_json) => {
                let token: SecretToken = serde_json::from_str(&secret_json).map_err(|e| {
                    DriftFsError::Serialization {
                        message: format!("failed to deserialize token from keyring: {e}"),
                        source: Some(Box::new(e)),
                    }
                })?;
                Ok(Some(token))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(DriftFsError::auth(format!(
                "failed to retrieve credentials from OS keyring: {e}"
            ))),
        }
    }

    fn delete_token(&self, account_id: &AccountId) -> Result<()> {
        let entry = self.entry_for(account_id)?;
        match entry.delete_credential() {
            Ok(_) | Err(keyring::Error::NoEntry) => {
                tracing::info!(account_id = %account_id, "deleted credentials from OS keyring");
                Ok(())
            }
            Err(e) => Err(DriftFsError::auth(format!(
                "failed to delete credentials from OS keyring: {e}"
            ))),
        }
    }
}

#[derive(Default)]
pub struct InMemoryCredentialStore {
    tokens: RwLock<HashMap<AccountId, SecretToken>>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn save_token(&self, account_id: &AccountId, token: &SecretToken) -> Result<()> {
        let mut map = self
            .tokens
            .write()
            .map_err(|_| DriftFsError::internal("credential store lock poisoned"))?;
        map.insert(account_id.clone(), token.clone());
        Ok(())
    }

    fn load_token(&self, account_id: &AccountId) -> Result<Option<SecretToken>> {
        let map = self
            .tokens
            .read()
            .map_err(|_| DriftFsError::internal("credential store lock poisoned"))?;
        Ok(map.get(account_id).cloned())
    }

    fn delete_token(&self, account_id: &AccountId) -> Result<()> {
        let mut map = self
            .tokens
            .write()
            .map_err(|_| DriftFsError::internal("credential store lock poisoned"))?;
        map.remove(account_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_store_lifecycle() {
        let store = InMemoryCredentialStore::new();
        let account_id = AccountId("acc-123".into());
        let token = SecretToken::new(
            "access_token_123".into(),
            Some("refresh_token_456".into()),
            "Bearer".into(),
            Some(3600),
            vec!["drive".into()],
        );

        assert!(store.load_token(&account_id).unwrap().is_none());

        store.save_token(&account_id, &token).unwrap();

        let loaded = store
            .load_token(&account_id)
            .unwrap()
            .expect("token loaded");
        assert_eq!(loaded.access_token, "access_token_123");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh_token_456"));

        store.delete_token(&account_id).unwrap();
        assert!(store.load_token(&account_id).unwrap().is_none());
    }
}

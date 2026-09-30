use crate::types::AccountIdentity;
use driftfs_core::{AccountId, DriftFsError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountRegistry {
    pub accounts: Vec<AccountIdentity>,
    pub default_account: Option<AccountId>,
}

pub struct AccountStore {
    file_path: PathBuf,
    cached: RwLock<AccountRegistry>,
}

impl AccountStore {
    pub fn new(file_path: PathBuf) -> Result<Self> {
        let registry = if file_path.exists() {
            let content =
                fs::read_to_string(&file_path).map_err(|e| DriftFsError::Configuration {
                    message: format!("failed to read account file {}: {e}", file_path.display()),
                })?;
            serde_json::from_str(&content).map_err(|e| DriftFsError::Serialization {
                message: format!("failed to parse accounts registry: {e}"),
                source: Some(Box::new(e)),
            })?
        } else {
            AccountRegistry::default()
        };

        Ok(Self {
            file_path,
            cached: RwLock::new(registry),
        })
    }

    pub fn in_default_dir() -> Result<Self> {
        let base_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        let driftfs_dir = base_dir.join("DriftFS");
        let accounts_file = driftfs_dir.join("accounts.json");
        Self::new(accounts_file)
    }

    fn save_to_disk(&self, registry: &AccountRegistry) -> Result<()> {
        if let Some(parent) = self.file_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| DriftFsError::Configuration {
                    message: format!(
                        "failed to create account store directory {}: {e}",
                        parent.display()
                    ),
                })?;
            }
        }

        let serialized =
            serde_json::to_string_pretty(registry).map_err(|e| DriftFsError::Serialization {
                message: format!("failed to serialize account registry: {e}"),
                source: Some(Box::new(e)),
            })?;

        // Atomic write-and-rename prevents partial writes on crash.
        let temp_path = self.file_path.with_extension("tmp");
        fs::write(&temp_path, serialized).map_err(|e| DriftFsError::Configuration {
            message: format!("failed to write temp account file: {e}"),
        })?;

        fs::rename(&temp_path, &self.file_path).map_err(|e| DriftFsError::Configuration {
            message: format!("failed to atomically persist accounts file: {e}"),
        })?;

        Ok(())
    }

    pub fn list_accounts(&self) -> Result<Vec<AccountIdentity>> {
        let reg = self
            .cached
            .read()
            .map_err(|_| DriftFsError::internal("account store lock poisoned"))?;
        Ok(reg.accounts.clone())
    }

    pub fn get_account(&self, id: &AccountId) -> Result<Option<AccountIdentity>> {
        let reg = self
            .cached
            .read()
            .map_err(|_| DriftFsError::internal("account store lock poisoned"))?;
        Ok(reg.accounts.iter().find(|a| &a.id == id).cloned())
    }

    pub fn save_account(&self, account: &AccountIdentity) -> Result<()> {
        let mut reg = self
            .cached
            .write()
            .map_err(|_| DriftFsError::internal("account store lock poisoned"))?;

        if let Some(idx) = reg.accounts.iter().position(|a| a.id == account.id) {
            reg.accounts[idx] = account.clone();
        } else {
            if reg.default_account.is_none() {
                reg.default_account = Some(account.id.clone());
            }
            reg.accounts.push(account.clone());
        }

        self.save_to_disk(&reg)?;
        tracing::info!(account_id = %account.id, email = %account.email, "persisted account identity");
        Ok(())
    }

    pub fn remove_account(&self, id: &AccountId) -> Result<()> {
        let mut reg = self
            .cached
            .write()
            .map_err(|_| DriftFsError::internal("account store lock poisoned"))?;

        reg.accounts.retain(|a| &a.id != id);
        if reg.default_account.as_ref() == Some(id) {
            reg.default_account = reg.accounts.first().map(|a| a.id.clone());
        }

        self.save_to_disk(&reg)?;
        tracing::info!(account_id = %id, "removed account identity from registry");
        Ok(())
    }

    pub fn default_account(&self) -> Result<Option<AccountId>> {
        let reg = self
            .cached
            .read()
            .map_err(|_| DriftFsError::internal("account store lock poisoned"))?;
        Ok(reg.default_account.clone())
    }

    pub fn set_default_account(&self, id: &AccountId) -> Result<()> {
        let mut reg = self
            .cached
            .write()
            .map_err(|_| DriftFsError::internal("account store lock poisoned"))?;

        if !reg.accounts.iter().any(|a| &a.id == id) {
            return Err(DriftFsError::not_found(format!("account {id}")));
        }

        reg.default_account = Some(id.clone());
        self.save_to_disk(&reg)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_store_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let store = AccountStore::new(path.clone()).unwrap();

        assert!(store.list_accounts().unwrap().is_empty());
        assert!(store.default_account().unwrap().is_none());

        let acc = AccountIdentity {
            id: AccountId("google-user-1".into()),
            provider: "google".into(),
            email: "user@example.com".into(),
            display_name: Some("Drift User".into()),
            provider_account_id: "sub-12345".into(),
            created_at: "2026-09-28T12:00:00Z".into(),
        };

        store.save_account(&acc).unwrap();
        assert_eq!(store.list_accounts().unwrap().len(), 1);
        assert_eq!(
            store.default_account().unwrap(),
            Some(AccountId("google-user-1".into()))
        );

        let reloaded = AccountStore::new(path).unwrap();
        let fetched = reloaded.get_account(&acc.id).unwrap().unwrap();
        assert_eq!(fetched, acc);

        reloaded.remove_account(&acc.id).unwrap();
        assert!(reloaded.list_accounts().unwrap().is_empty());
        assert!(reloaded.default_account().unwrap().is_none());
    }
}

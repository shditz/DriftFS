mod client;
mod mapping;
mod provider;

pub use client::DriveClientConfig;
pub use mapping::{
    drive_file_to_metadata, DriveAbout, DriveAboutUser, DriveFile, DriveStorageQuota,
    GOOGLE_DRIVE_FOLDER_MIME,
};
pub use provider::GoogleDriveProvider;

#[cfg(test)]
mod tests {
    use super::*;
    use driftfs_auth::TokenProvider;
    use driftfs_core::{AccountId, Result};
    use driftfs_provider::CloudProvider;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    struct StaticTokenProvider {
        account_id: AccountId,
        token: String,
    }

    impl TokenProvider for StaticTokenProvider {
        fn get_access_token<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
            Box::pin(async move { Ok(self.token.clone()) })
        }

        fn account_id(&self) -> &AccountId {
            &self.account_id
        }
    }

    #[tokio::test]
    async fn provider_account_id_matches_token_provider() {
        let tp = Arc::new(StaticTokenProvider {
            account_id: AccountId("google:user_test_999".into()),
            token: "dummy_bearer_token".into(),
        });

        let provider = GoogleDriveProvider::new(tp);
        let id = provider.account_id().await.unwrap();
        assert_eq!(id.0, "google:user_test_999");
    }
}

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use driftfs_auth::{
    AccountStore, AuthService, GoogleOAuthClient, KeyringCredentialStore, OAuthConfig,
};
use driftfs_cache::{BoundedChunkCache, DEFAULT_CHUNK_SIZE};
use driftfs_config::DriftFsConfig;
use driftfs_core::{AccountId, DriftFsError, FileId, Result};
use driftfs_filesystem::DriftFsVfs;
use driftfs_google_drive::GoogleDriveProvider;
use driftfs_metadata::MetadataStore;
use driftfs_platform_windows::{find_available_drive_letter, DriftFsMount, MountConfig};
use driftfs_sync::{SyncEngine, SyncWorker};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

pub struct AppState {
    pub is_authenticated: bool,
    pub account_email: String,
    pub quota_used: u64,
    pub quota_total: u64,

    pub is_mounted: bool,
    pub drive_letter: String,
    pub mount_status_text: String,

    pub sync_status_text: String,
    pub last_sync_time: String,
    pub dirty_files_count: usize,

    pub cache_used_bytes: u64,
    pub cache_max_bytes: u64,
    pub cache_dir: PathBuf,

    pub auto_start: bool,
    pub start_minimized: bool,
    pub prefetch_enabled: bool,
    pub sync_interval_secs: u64,

    pub activity_log: Vec<String>,
}

pub struct AppController {
    auth_service: Arc<AuthService>,
    state: Mutex<AppState>,
    active_mount: Mutex<Option<DriftFsMount>>,
    sync_cancel: Mutex<Option<CancellationToken>>,
    chunk_cache: Mutex<Option<Arc<BoundedChunkCache>>>,
    rt_handle: tokio::runtime::Handle,
    is_busy: Arc<AtomicBool>,
    is_mounted_cache: Arc<AtomicBool>,
    cached_drive_letter: std::sync::RwLock<String>,
}

impl AppController {
    pub async fn new(rt_handle: tokio::runtime::Handle) -> Result<Arc<Self>> {
        let config_path = DriftFsConfig::default_path();
        let config = DriftFsConfig::load(&config_path).unwrap_or_default();

        let base_data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("DriftFS");
        let cache_dir = config.cache.directory.clone();
        let _ = std::fs::create_dir_all(&base_data_dir);
        let _ = std::fs::create_dir_all(&cache_dir);

        let accounts = Arc::new(AccountStore::in_default_dir()?);
        let credentials = Arc::new(KeyringCredentialStore::default());
        let oauth = GoogleOAuthClient::new(OAuthConfig::default());
        let auth_service = Arc::new(AuthService::new(oauth, credentials, accounts));

        let initial_drive = config
            .mount
            .mount_point
            .chars()
            .next()
            .map(|c| format!("{c}:"))
            .unwrap_or_else(|| "G:".to_string());

        let initial_state = AppState {
            is_authenticated: false,
            account_email: "Not Connected".into(),
            quota_used: 0,
            quota_total: 0,

            is_mounted: false,
            drive_letter: initial_drive.clone(),
            mount_status_text: "Unmounted".into(),

            sync_status_text: "Idle".into(),
            last_sync_time: "Not yet synced".into(),
            dirty_files_count: 0,

            cache_used_bytes: 0,
            cache_max_bytes: config.cache.max_size_bytes,
            cache_dir: cache_dir.clone(),

            auto_start: Self::check_windows_autostart(),
            start_minimized: true,
            prefetch_enabled: config.network.prefetch_enabled,
            sync_interval_secs: config.sync.poll_interval_secs,

            activity_log: vec![
                "DriftFS initialized".into(),
                "Ready to mount Google Drive".into(),
            ],
        };

        let cached_drive_letter = std::sync::RwLock::new(initial_drive);

        let controller = Arc::new(Self {
            auth_service,
            state: Mutex::new(initial_state),
            active_mount: Mutex::new(None),
            sync_cancel: Mutex::new(None),
            chunk_cache: Mutex::new(None),
            rt_handle,
            is_busy: Arc::new(AtomicBool::new(false)),
            is_mounted_cache: Arc::new(AtomicBool::new(false)),
            cached_drive_letter,
        });

        controller.init_cache().await;
        controller.check_existing_credentials().await;

        Ok(controller)
    }

    pub fn is_mounted(&self) -> bool {
        self.is_mounted_cache.load(Ordering::Relaxed)
    }

    pub fn current_drive_letter(&self) -> String {
        self.cached_drive_letter
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "G:".to_string())
    }

    pub async fn state(&self) -> AppState {
        let guard = self.state.lock().await;
        AppState {
            is_authenticated: guard.is_authenticated,
            account_email: guard.account_email.clone(),
            quota_used: guard.quota_used,
            quota_total: guard.quota_total,

            is_mounted: guard.is_mounted,
            drive_letter: guard.drive_letter.clone(),
            mount_status_text: guard.mount_status_text.clone(),

            sync_status_text: guard.sync_status_text.clone(),
            last_sync_time: guard.last_sync_time.clone(),
            dirty_files_count: guard.dirty_files_count,

            cache_used_bytes: guard.cache_used_bytes,
            cache_max_bytes: guard.cache_max_bytes,
            cache_dir: guard.cache_dir.clone(),

            auto_start: guard.auto_start,
            start_minimized: guard.start_minimized,
            prefetch_enabled: guard.prefetch_enabled,
            sync_interval_secs: guard.sync_interval_secs,

            activity_log: guard.activity_log.clone(),
        }
    }

    pub async fn add_log_entry(&self, msg: impl Into<String>) {
        let entry = msg.into();
        info!(target: "driftfs::activity", "{}", entry);
        let mut state = self.state.lock().await;
        state.activity_log.push(entry);
        if state.activity_log.len() > 50 {
            state.activity_log.remove(0);
        }
    }

    async fn init_cache(&self) {
        let (dir, max_size) = {
            let state = self.state.lock().await;
            (state.cache_dir.clone(), state.cache_max_bytes)
        };

        match BoundedChunkCache::new(dir, max_size, DEFAULT_CHUNK_SIZE) {
            Ok(cache) => {
                let current = cache.current_size();
                let mut state = self.state.lock().await;
                state.cache_used_bytes = current;
                let arc_cache = Arc::new(cache);
                *self.chunk_cache.lock().await = Some(arc_cache);
            }
            Err(e) => {
                warn!(?e, "failed to initialize chunk cache");
            }
        }
    }

    pub async fn check_existing_credentials(&self) {
        let accounts = self
            .auth_service
            .accounts()
            .list_accounts()
            .unwrap_or_default();
        if let Some(account) = accounts.first() {
            if let Ok(Some(_)) = self.auth_service.credentials().load_token(&account.id) {
                let mut state = self.state.lock().await;
                state.is_authenticated = true;
                state.account_email = account.email.clone();
                drop(state);

                self.add_log_entry(format!("Connected to Google Account: {}", account.email))
                    .await;
                self.refresh_drive_quota(&account.id).await;
            }
        }
    }

    pub async fn refresh_drive_quota(&self, account_id: &AccountId) {
        let token_provider = self.auth_service.create_token_provider(account_id.clone());
        let provider = GoogleDriveProvider::new(token_provider);

        if let Ok(about) = provider.about().await {
            if let Some(quota) = about.storage_quota {
                let used = quota
                    .usage
                    .as_deref()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let total = quota
                    .limit
                    .as_deref()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(15 * 1024 * 1024 * 1024);
                let mut state = self.state.lock().await;
                state.quota_used = used;
                state.quota_total = total;
            }
        }
    }

    pub async fn start_oauth_flow(self: &Arc<Self>) -> Result<()> {
        if self.is_busy.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        self.add_log_entry("Starting Google sign-in...").await;
        let session = match self.auth_service.start_login_flow().await {
            Ok(s) => s,
            Err(e) => {
                self.is_busy.store(false, Ordering::SeqCst);
                self.add_log_entry(format!("Sign-in failed to start: {e}"))
                    .await;
                return Err(e);
            }
        };

        if let Err(e) = open::that(&session.auth_url) {
            warn!(
                ?e,
                "failed to open browser automatically; URL: {}", session.auth_url
            );
        }

        let auth_service = Arc::clone(&self.auth_service);
        let is_busy = Arc::clone(&self.is_busy);
        let this = Arc::clone(self);

        self.rt_handle.spawn(async move {
            match auth_service
                .complete_login_flow(session, Duration::from_secs(180))
                .await
            {
                Ok(identity) => {
                    {
                        let mut state = this.state.lock().await;
                        state.is_authenticated = true;
                        state.account_email = identity.email.clone();
                    }
                    this.add_log_entry(format!("Signed in as: {}", identity.email))
                        .await;
                    this.refresh_drive_quota(&identity.id).await;
                }
                Err(e) => {
                    error!(?e, "OAuth login flow error");
                    this.add_log_entry(format!("Sign-in failed: {e}")).await;
                }
            }
            is_busy.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    pub async fn logout(&self) -> Result<()> {
        if self.is_mounted() {
            self.unmount().await?;
        }

        let accounts = self
            .auth_service
            .accounts()
            .list_accounts()
            .unwrap_or_default();
        for acc in accounts {
            let _ = self.auth_service.credentials().delete_token(&acc.id);
            let _ = self.auth_service.accounts().remove_account(&acc.id);
        }

        let mut state = self.state.lock().await;
        state.is_authenticated = false;
        state.account_email = "Not Connected".into();
        state.quota_used = 0;
        state.quota_total = 0;
        drop(state);

        self.add_log_entry("Disconnected Google Account").await;
        Ok(())
    }

    pub async fn mount(&self) -> Result<()> {
        if self.is_busy.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let account = match self
            .auth_service
            .accounts()
            .list_accounts()
            .unwrap_or_default()
            .into_iter()
            .next()
        {
            Some(a) => a,
            None => {
                self.is_busy.store(false, Ordering::SeqCst);
                return Err(DriftFsError::auth("No authenticated account to mount"));
            }
        };

        {
            let mut state = self.state.lock().await;
            state.mount_status_text = "Mounting...".into();
        }

        let preferred_letter = self
            .state
            .lock()
            .await
            .drive_letter
            .chars()
            .next()
            .unwrap_or('G');

        let available_letter = match find_available_drive_letter(Some(preferred_letter)) {
            Ok(c) => c,
            Err(_) => match find_available_drive_letter(None) {
                Ok(c) => c,
                Err(e) => {
                    self.is_busy.store(false, Ordering::SeqCst);
                    let mut state = self.state.lock().await;
                    state.mount_status_text = "Drive Unavailable".into();
                    let err_msg = format!("Drive letter {preferred_letter}: is in use. Choose another letter in Settings.");
                    drop(state);
                    self.add_log_entry(&err_msg).await;
                    return Err(DriftFsError::Filesystem {
                        message: format!("No drive letter available: {e}"),
                        source: None,
                    });
                }
            },
        };

        let base_data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("DriftFS");
        let db_path = base_data_dir.join("metadata.db");
        let staging_path = base_data_dir.join("staging");

        let store = match MetadataStore::open(&db_path) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                self.is_busy.store(false, Ordering::SeqCst);
                self.add_log_entry(format!("Metadata database error: {e}"))
                    .await;
                return Err(e.into());
            }
        };

        let token_provider = self.auth_service.create_token_provider(account.id.clone());
        let provider = Arc::new(GoogleDriveProvider::new(token_provider));

        let mut vfs = match DriftFsVfs::new(store.clone(), provider.clone(), staging_path) {
            Ok(v) => v,
            Err(e) => {
                self.is_busy.store(false, Ordering::SeqCst);
                self.add_log_entry(format!("VFS initialization failed: {e}"))
                    .await;
                return Err(DriftFsError::Filesystem {
                    message: e.to_string(),
                    source: None,
                });
            }
        };

        vfs = vfs.with_root_folder_id(FileId("root".into()));

        if let Some(ref cache) = *self.chunk_cache.lock().await {
            vfs = vfs.with_chunk_cache(Arc::clone(cache));
        }

        let prefetch = self.state.lock().await.prefetch_enabled;
        vfs = vfs.with_prefetch(prefetch);
        let arc_vfs = Arc::new(vfs);

        let mount_config = MountConfig {
            drive_letter: Some(available_letter),
            volume_label: "Google Drive".into(),
        };

        // Bootstrap root hierarchy before mounting so Explorer sees items immediately
        let sync_engine = Arc::new(SyncEngine::new(provider.clone(), store.clone(), account.id));
        if let Err(e) = sync_engine.bootstrap_root().await {
            tracing::warn!(error = %e, "root bootstrap failed or partially completed; proceeding with mount");
        }

        let mount_res = DriftFsMount::mount(arc_vfs, self.rt_handle.clone(), mount_config);
        let drift_mount = match mount_res {
            Ok(m) => m,
            Err(e) => {
                self.is_busy.store(false, Ordering::SeqCst);
                let err_msg = format!("Mount failed: {e}. Check that WinFsp is installed.");
                self.add_log_entry(&err_msg).await;
                let mut state = self.state.lock().await;
                state.mount_status_text = "Mount Failed".into();
                return Err(DriftFsError::Filesystem {
                    message: err_msg,
                    source: None,
                });
            }
        };

        let sync_cancel = CancellationToken::new();
        let poll_interval = Duration::from_secs(self.state.lock().await.sync_interval_secs);
        let sync_worker = SyncWorker::new(sync_engine, poll_interval, sync_cancel.clone());

        self.rt_handle.spawn(async move {
            sync_worker.run().await;
        });

        *self.active_mount.lock().await = Some(drift_mount);
        *self.sync_cancel.lock().await = Some(sync_cancel);
        self.is_mounted_cache.store(true, Ordering::Relaxed);

        let drive_str = format!("{available_letter}:");
        if let Ok(mut g) = self.cached_drive_letter.write() {
            *g = drive_str.clone();
        }
        {
            let mut state = self.state.lock().await;
            state.is_mounted = true;
            state.drive_letter = drive_str.clone();
            state.mount_status_text = format!("Mounted on {drive_str}");
            state.sync_status_text = "Up to date".into();
        }

        self.add_log_entry(format!("Virtual drive mounted at {drive_str}"))
            .await;
        self.is_busy.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub async fn unmount(&self) -> Result<()> {
        if let Some(cancel) = self.sync_cancel.lock().await.take() {
            cancel.cancel();
        }

        let mut mount_guard = self.active_mount.lock().await;
        if let Some(mut mount) = mount_guard.take() {
            mount.unmount();
        }

        self.is_mounted_cache.store(false, Ordering::Relaxed);

        let mut state = self.state.lock().await;
        state.is_mounted = false;
        state.mount_status_text = "Unmounted".into();
        state.sync_status_text = "Idle".into();
        let drive = state.drive_letter.clone();
        drop(state);

        self.add_log_entry(format!("Virtual drive {drive} unmounted"))
            .await;
        Ok(())
    }

    pub async fn purge_cache(&self) -> Result<()> {
        let cache_dir = self.state.lock().await.cache_dir.clone();

        *self.chunk_cache.lock().await = None;

        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("chunk") {
                    let _ = std::fs::remove_file(path);
                }
            }
        }

        self.init_cache().await;
        self.add_log_entry("Cleared local cache").await;
        Ok(())
    }

    pub async fn update_metrics(&self) {
        if let Some(ref cache) = *self.chunk_cache.lock().await {
            let used = cache.current_size();
            let mut state = self.state.lock().await;
            state.cache_used_bytes = used;
        }
    }

    pub fn open_explorer(&self, drive_letter: &str) {
        let path = format!("{drive_letter}\\");
        let _ = open::that(path);
    }

    pub async fn save_settings(&self) {
        let state = self.state.lock().await;
        let config_path = DriftFsConfig::default_path();
        let mut config = DriftFsConfig::load(&config_path).unwrap_or_default();
        config.network.prefetch_enabled = state.prefetch_enabled;
        config.mount.mount_point = state.drive_letter.clone();
        let _ = config.save(&config_path);
    }

    fn check_windows_autostart() -> bool {
        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            let output = Command::new("reg")
                .args([
                    "query",
                    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                    "/v",
                    "DriftFS",
                ])
                .output();
            matches!(output, Ok(out) if out.status.success())
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    fn update_windows_autostart(enabled: bool) {
        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            if enabled {
                if let Ok(exe) = std::env::current_exe() {
                    let exe_str = format!("\"{}\"", exe.display());
                    let _ = Command::new("reg")
                        .args([
                            "add",
                            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                            "/v",
                            "DriftFS",
                            "/t",
                            "REG_SZ",
                            "/d",
                            &exe_str,
                            "/f",
                        ])
                        .output();
                }
            } else {
                let _ = Command::new("reg")
                    .args([
                        "delete",
                        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                        "/v",
                        "DriftFS",
                        "/f",
                    ])
                    .output();
            }
        }
    }

    pub async fn toggle_prefetch(&self) {
        let mut state = self.state.lock().await;
        state.prefetch_enabled = !state.prefetch_enabled;
        let enabled = state.prefetch_enabled;
        drop(state);
        self.save_settings().await;
        self.add_log_entry(format!("Smart pre-fetching set to: {enabled}"))
            .await;
    }

    pub async fn toggle_start_minimized(&self) {
        let mut state = self.state.lock().await;
        state.start_minimized = !state.start_minimized;
        drop(state);
        self.save_settings().await;
    }

    pub async fn toggle_autostart(&self) {
        let mut state = self.state.lock().await;
        state.auto_start = !state.auto_start;
        let enabled = state.auto_start;
        drop(state);
        Self::update_windows_autostart(enabled);
        self.save_settings().await;
        self.add_log_entry(format!("Windows startup set to: {enabled}"))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_controller_initial_state_and_toggles() {
        let rt_handle = tokio::runtime::Handle::current();
        let controller = AppController::new(rt_handle)
            .await
            .expect("controller should initialize");

        assert!(!controller.is_mounted());
        let initial_state = controller.state().await;
        assert!(!initial_state.is_mounted);

        let initial_prefetch = initial_state.prefetch_enabled;
        controller.toggle_prefetch().await;
        assert_eq!(controller.state().await.prefetch_enabled, !initial_prefetch);

        let initial_minimized = initial_state.start_minimized;
        controller.toggle_start_minimized().await;
        assert_eq!(controller.state().await.start_minimized, !initial_minimized);

        let initial_autostart = initial_state.auto_start;
        controller.toggle_autostart().await;
        assert_eq!(controller.state().await.auto_start, !initial_autostart);

        controller.add_log_entry("Test log message").await;
        let logs = controller.state().await.activity_log;
        assert!(logs.iter().any(|entry| entry.contains("Test log message")));
    }
}

mod controller;
mod tray;

use std::sync::Arc;
use std::time::Duration;

use controller::AppController;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tray::{SystemTrayManager, TrayCommand};

slint::include_modules!();

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    driftfs_logging::init("info");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start Tokio runtime");

    let controller = rt.block_on(async {
        AppController::new(rt.handle().clone())
            .await
            .expect("failed to initialize AppController")
    });

    let tray = match SystemTrayManager::new() {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(?e, "system tray could not be initialized");
            None
        }
    };

    let app = AppWindow::new()?;
    let app_weak = app.as_weak();

    {
        let ctrl = Arc::clone(&controller);
        let rt_handle = rt.handle().clone();
        app.on_login_clicked(move || {
            let c = Arc::clone(&ctrl);
            rt_handle.spawn(async move {
                let _ = c.start_oauth_flow().await;
            });
        });
    }

    {
        let ctrl = Arc::clone(&controller);
        let rt_handle = rt.handle().clone();
        app.on_logout_clicked(move || {
            let c = Arc::clone(&ctrl);
            rt_handle.spawn(async move {
                let _ = c.logout().await;
            });
        });
    }

    {
        let ctrl = Arc::clone(&controller);
        let rt_handle = rt.handle().clone();
        app.on_mount_toggle_clicked(move || {
            let c = Arc::clone(&ctrl);
            rt_handle.spawn(async move {
                let is_mounted = c.state().await.is_mounted;
                if is_mounted {
                    let _ = c.unmount().await;
                } else {
                    let _ = c.mount().await;
                }
            });
        });
    }

    {
        let ctrl = Arc::clone(&controller);
        app.on_open_explorer_clicked(move || {
            let drive = ctrl.current_drive_letter();
            ctrl.open_explorer(&drive);
        });
    }

    {
        let ctrl = Arc::clone(&controller);
        let rt_handle = rt.handle().clone();
        app.on_purge_cache_clicked(move || {
            let c = Arc::clone(&ctrl);
            rt_handle.spawn(async move {
                let _ = c.purge_cache().await;
            });
        });
    }

    {
        let ctrl = Arc::clone(&controller);
        let rt_handle = rt.handle().clone();
        app.on_toggle_prefetch(move || {
            let c = Arc::clone(&ctrl);
            rt_handle.spawn(async move {
                c.toggle_prefetch().await;
            });
        });
    }

    {
        let ctrl = Arc::clone(&controller);
        let rt_handle = rt.handle().clone();
        app.on_toggle_autostart(move || {
            let c = Arc::clone(&ctrl);
            rt_handle.spawn(async move {
                c.toggle_autostart().await;
            });
        });
    }

    {
        let ctrl = Arc::clone(&controller);
        let rt_handle = rt.handle().clone();
        app.on_toggle_start_minimized(move || {
            let c = Arc::clone(&ctrl);
            rt_handle.spawn(async move {
                c.toggle_start_minimized().await;
            });
        });
    }

    let timer = slint::Timer::default();
    let poll_app = app_weak.clone();
    let poll_ctrl = Arc::clone(&controller);
    let poll_tray = tray;
    let poll_rt = rt.handle().clone();

    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(400),
        move || {
            if let Some(ref t) = poll_tray {
                while let Some(cmd) = t.try_recv() {
                    match cmd {
                        TrayCommand::ShowWindow => {
                            if let Some(w) = poll_app.upgrade() {
                                w.show().unwrap_or_default();
                            }
                        }
                        TrayCommand::ToggleMount => {
                            let c = Arc::clone(&poll_ctrl);
                            poll_rt.spawn(async move {
                                let is_mounted = c.state().await.is_mounted;
                                if is_mounted {
                                    let _ = c.unmount().await;
                                } else {
                                    let _ = c.mount().await;
                                }
                            });
                        }
                        TrayCommand::OpenExplorer => {
                            let drive = poll_ctrl.current_drive_letter();
                            poll_ctrl.open_explorer(&drive);
                        }
                        TrayCommand::Quit => {
                            let c = Arc::clone(&poll_ctrl);
                            poll_rt.spawn(async move {
                                let _ = c.unmount().await;
                                slint::invoke_from_event_loop(|| {
                                    slint::quit_event_loop().unwrap_or_default();
                                })
                                .unwrap_or_default();
                            });
                        }
                    }
                }
                let tray_drive = poll_ctrl.current_drive_letter();
                t.update_mount_status(poll_ctrl.is_mounted(), &tray_drive);
            }

            let c = Arc::clone(&poll_ctrl);
            let ui_weak = poll_app.clone();

            poll_rt.spawn(async move {
                c.update_metrics().await;
                let state = c.state().await;

                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_is_authenticated(state.is_authenticated);
                        ui.set_account_email(SharedString::from(&state.account_email));

                        let quota_pct = if state.quota_total > 0 {
                            state.quota_used as f32 / state.quota_total as f32
                        } else {
                            0.0
                        };
                        ui.set_quota_percentage(quota_pct);
                        ui.set_quota_used_str(SharedString::from(format_bytes(state.quota_used)));
                        ui.set_quota_total_str(SharedString::from(format_bytes(state.quota_total)));

                        ui.set_is_mounted(state.is_mounted);
                        ui.set_drive_letter(SharedString::from(&state.drive_letter));
                        ui.set_mount_status_text(SharedString::from(&state.mount_status_text));

                        ui.set_sync_status_text(SharedString::from(&state.sync_status_text));
                        ui.set_last_sync_time(SharedString::from(&state.last_sync_time));
                        ui.set_dirty_files_count(state.dirty_files_count as i32);

                        let cache_pct = if state.cache_max_bytes > 0 {
                            state.cache_used_bytes as f32 / state.cache_max_bytes as f32
                        } else {
                            0.0
                        };
                        ui.set_cache_percentage(cache_pct);
                        ui.set_cache_used_str(SharedString::from(format_bytes(
                            state.cache_used_bytes,
                        )));
                        ui.set_cache_max_str(SharedString::from(format_bytes(
                            state.cache_max_bytes,
                        )));
                        ui.set_cache_dir_path(SharedString::from(
                            state.cache_dir.to_string_lossy().to_string(),
                        ));

                        ui.set_auto_start(state.auto_start);
                        ui.set_start_minimized(state.start_minimized);
                        ui.set_prefetch_enabled(state.prefetch_enabled);
                        ui.set_sync_interval_secs(state.sync_interval_secs as i32);

                        let log_models: Vec<SharedString> = state
                            .activity_log
                            .into_iter()
                            .rev()
                            .take(15)
                            .map(SharedString::from)
                            .collect();
                        let model = ModelRc::new(VecModel::from(log_models));
                        ui.set_activity_entries(model);
                    }
                });
            });
        },
    );

    app.run()?;

    rt.block_on(async {
        let _ = controller.unmount().await;
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes_boundaries() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(1073741824), "1.0 GB");
        assert_eq!(format_bytes(5368709120), "5.0 GB");
    }
}

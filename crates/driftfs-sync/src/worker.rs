use crate::engine::SyncEngine;
use driftfs_provider::CloudProvider;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct SyncWorker<P: CloudProvider> {
    engine: Arc<SyncEngine<P>>,
    poll_interval: Duration,
    cancellation_token: CancellationToken,
}

impl<P: CloudProvider> SyncWorker<P> {
    pub fn new(
        engine: Arc<SyncEngine<P>>,
        poll_interval: Duration,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            engine,
            poll_interval,
            cancellation_token,
        }
    }

    pub async fn run(&self) {
        let mut sleep_duration = Duration::from_millis(500);
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(60);

        loop {
            tokio::select! {
                _ = self.cancellation_token.cancelled() => {
                    tracing::debug!("sync worker received cancellation, exiting cleanly");
                    break;
                }
                _ = tokio::time::sleep(sleep_duration) => {
                    let outbound_result = self.engine.process_outbound_queue().await;
                    let outbound_count = match outbound_result {
                        Ok(count) => {
                            if count > 0 {
                                tracing::debug!(count, "outbound sync processed items");
                            }
                            count
                        }
                        Err(ref e) => {
                            tracing::warn!(?e, "outbound sync error");
                            0
                        }
                    };

                    let inbound_result = self.engine.sync_changes().await;
                    let inbound_count = match inbound_result {
                        Ok(count) => {
                            if count > 0 {
                                tracing::debug!(count, "sync cycle applied remote changes");
                            }
                            count
                        }
                        Err(ref e) => {
                            tracing::warn!(?e, ?backoff, "sync cycle encountered error, backing off");
                            0
                        }
                    };

                    let has_pending = self
                        .engine
                        .store()
                        .get_pending_sync_stats()
                        .map(|(c, _)| c > 0)
                        .unwrap_or(false);

                    if outbound_count > 0 || has_pending {
                        sleep_duration = Duration::from_millis(250);
                        backoff = Duration::from_secs(1);
                    } else if inbound_count > 0 {
                        sleep_duration = Duration::from_secs(1);
                        backoff = Duration::from_secs(1);
                    } else if outbound_result.is_err() || inbound_result.is_err() {
                        sleep_duration = backoff;
                        backoff = std::cmp::min(backoff * 2, max_backoff);
                    } else {
                        sleep_duration = self.poll_interval;
                        backoff = Duration::from_secs(1);
                    }
                }
            }
        }
    }
}

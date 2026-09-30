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
        let mut sleep_duration = self.poll_interval;
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(60);

        loop {
            tokio::select! {
                _ = self.cancellation_token.cancelled() => {
                    tracing::debug!("sync worker received cancellation, exiting cleanly");
                    break;
                }
                _ = tokio::time::sleep(sleep_duration) => {
                    match self.engine.sync_changes().await {
                        Ok(count) => {
                            if count > 0 {
                                tracing::debug!(count, "sync cycle applied remote changes");
                            }
                            backoff = Duration::from_secs(1);
                            sleep_duration = self.poll_interval;
                        }
                        Err(e) => {
                            tracing::warn!(?e, ?backoff, "sync cycle encountered error, backing off");
                            sleep_duration = backoff;
                            backoff = std::cmp::min(backoff * 2, max_backoff);
                        }
                    }
                }
            }
        }
    }
}

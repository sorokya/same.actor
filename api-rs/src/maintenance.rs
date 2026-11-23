//! Retention, on a timer.
//!
//! Runs at startup and then hourly. Hourly rather than nightly on purpose: a container that was
//! down over the scheduled hour would otherwise skip a day, and the work is idempotent — a run
//! with nothing to do is one cheap query.

use std::time::Duration;

use crate::state::AppState;

const INTERVAL: Duration = Duration::from_secs(3600);

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            match state.store.maintain(state.config.log_retention_days).await {
                Ok(report) if !report.is_empty() => tracing::info!(
                    days = report.days.len(),
                    rows = report.rows_deleted,
                    bans = report.bans_purged,
                    "rolled up and pruned old rows"
                ),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "retention failed; will retry next hour"),
            }

            tokio::time::sleep(INTERVAL).await;
        }
    });
}

use std::{sync::Arc, time::Duration};

use crate::AppState;

pub fn spawn(state: Arc<AppState>) {
    let interval_secs = state.config.signin_cleanup_interval_secs;
    if interval_secs == 0 {
        tracing::info!("signin challenge cleanup disabled");
        return;
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            if let Err(err) = run_once(&state).await {
                tracing::error!(error = %err, "signin challenge cleanup failed");
            }
        }
    });
}

async fn run_once(state: &AppState) -> anyhow::Result<u64> {
    let now = unix_now();
    let cutoff = now - state.config.signin_cleanup_retention_secs;
    let deleted = state.db.purge_signin_challenges(cutoff).await?;
    if deleted > 0 {
        tracing::info!(deleted, cutoff, "signin challenge cleanup done");
    }
    Ok(deleted)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after unix epoch")
        .as_secs() as i64
}

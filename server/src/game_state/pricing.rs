//! Pricing system phase 1 (doc/PRICING.md): hourly gold supply snapshots.

use std::sync::Arc;

use tracing::{info, warn};

use crate::auth::{unix_now, AuthService};
use crate::world_config::world_config;

impl super::GameState {
    /// Called right after the dirty-save flush; the DB ignores repeats
    /// within the same hour.
    pub async fn tick_gold_snapshot(&self, auth: &Arc<AuthService>) {
        let auth = Arc::clone(auth);
        let active_days = world_config().pricing.active_days;
        match super::auth_db(move || auth.record_gold_snapshot(unix_now(), active_days)).await {
            Ok(Some(ts)) => info!("gold snapshot: recorded hour {ts}"),
            Ok(None) => {}
            Err(err) => warn!("gold snapshot: {err}"),
        }
    }
}

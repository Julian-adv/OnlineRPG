//! Pricing system (doc/PRICING.md): hourly gold snapshots and the price
//! meeting on Serin's dark evening that moves the merchant price index.

use std::sync::Arc;

use tracing::{info, warn};

use onlinerpg_shared::moon::{days_until_serin_dark, is_serin_dark_day};
use onlinerpg_shared::pricing::{PricingNotice, Trend};

use crate::auth::{unix_now, AuthService, PricingMeeting};
use crate::celestial::is_after_sunset;
use crate::types::{PlayerId, ServerMessage};
use crate::world_config::{world_config, PricingConfig};

/// Fractional step a meeting would take (doc/PRICING.md 조정 공식).
fn index_step(growth: f64, elapsed_days: i64, cfg: &PricingConfig) -> f64 {
    let target = cfg.target_daily_growth * elapsed_days.max(1) as f64;
    (cfg.gain * (growth - target)).clamp(-cfg.max_step_per_meeting, cfg.max_step_per_meeting)
}

pub(crate) fn next_index_percent(
    current: u32,
    growth: f64,
    elapsed_days: i64,
    cfg: &PricingConfig,
) -> u32 {
    let step = index_step(growth, elapsed_days, cfg);
    let index = (f64::from(current) / 100.0 * (1.0 + step)).clamp(cfg.index_min, cfg.index_max);
    (index * 100.0).round() as u32
}

impl super::GameState {
    pub async fn load_pricing(&self, auth: &Arc<AuthService>) {
        let auth = Arc::clone(auth);
        match super::auth_db(move || auth.load_pricing_state()).await {
            Ok(state) => {
                info!("pricing: index {}%", state.index_percent);
                *self.pricing.write().await = state;
            }
            Err(err) => warn!("pricing: failed to load state, using defaults: {err}"),
        }
    }

    pub async fn price_index_percent(&self) -> u32 {
        self.pricing.read().await.index_percent
    }

    #[cfg(test)]
    pub(crate) async fn set_price_index_percent(&self, percent: u32) {
        self.pricing.write().await.index_percent = percent;
    }

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

    /// The market picture for NPC roleplay; the trend projects the next
    /// meeting's step from the latest hourly snapshot.
    pub async fn pricing_notice(&self, auth: &Arc<AuthService>) -> ServerMessage {
        let state = self.pricing.read().await.clone();
        let game_day = self.current_game_day();
        let reader = Arc::clone(auth);
        let (last_change_pct, reading) = super::auth_db(move || reader.pricing_notice_inputs())
            .await
            .unwrap_or_else(|err| {
                warn!("pricing: notice fell back to defaults: {err}");
                (0, None)
            });

        let step = match (state.m_prev, state.last_meeting_day, reading) {
            (Some(prev), Some(last), Some(now)) if prev > 0.0 => index_step(
                (now - prev) / prev,
                game_day - last,
                &world_config().pricing,
            ),
            _ => 0.0,
        };
        let trend = if step > 0.01 {
            Trend::Rising
        } else if step < -0.01 {
            Trend::Falling
        } else {
            Trend::Steady
        };
        let mut meeting_in_days = days_until_serin_dark(game_day);
        if meeting_in_days == 0 && state.last_meeting_day == Some(game_day) {
            meeting_in_days = days_until_serin_dark(game_day + 1) + 1;
        }
        ServerMessage::PricingNotice(PricingNotice {
            index_percent: state.index_percent,
            last_change_pct,
            trend,
            meeting_in_days,
        })
    }

    async fn send_pricing_notice_to_npcs(&self, auth: &Arc<AuthService>) {
        let npcs: Vec<PlayerId> = {
            let players = self.players.read().await;
            players
                .values()
                .filter(|p| p.is_official_npc)
                .map(|p| p.id)
                .collect()
        };
        if npcs.is_empty() {
            return;
        }
        let notice = self.pricing_notice(auth).await;
        self.send_direct_message_to_players(&npcs, notice).await;
    }

    /// Holds the merchants' price meeting once per Serin dark evening. The
    /// first meeting (or one with nobody active) only records the reading.
    pub async fn tick_pricing_meeting(&self, auth: &Arc<AuthService>) {
        let datetime = self.current_game_datetime();
        let game_day = self.current_game_day();
        if !is_serin_dark_day(game_day) || !is_after_sunset(&datetime) {
            return;
        }
        let mut state = self.pricing.write().await;
        if state.last_meeting_day.is_some_and(|day| day >= game_day) {
            return;
        }

        let cfg = &world_config().pricing;
        let reader = Arc::clone(auth);
        let m_now = match super::auth_db(move || {
            reader.active_gold_per_character(unix_now(), cfg.active_days)
        })
        .await
        {
            Ok(m) => m,
            Err(err) => return warn!("pricing: meeting skipped, DB read failed: {err}"),
        };

        let meeting = m_now.and_then(|now| {
            let (prev, last) = state.m_prev.zip(state.last_meeting_day)?;
            (prev > 0.0).then(|| {
                let growth = (now - prev) / prev;
                PricingMeeting {
                    game_day,
                    m_prev: prev,
                    m_now: now,
                    growth,
                    index_before: state.index_percent,
                    index_after: next_index_percent(
                        state.index_percent,
                        growth,
                        game_day - last,
                        cfg,
                    ),
                }
            })
        });
        if let Some(m) = &meeting {
            state.index_percent = m.index_after;
            info!(
                "pricing: meeting on day {game_day}: gold/active {:.1} -> {:.1} ({:+.1}%), index {}% -> {}%",
                m.m_prev, m.m_now, m.growth * 100.0, m.index_before, m.index_after
            );
        } else {
            info!("pricing: meeting on day {game_day} recorded reading {m_now:?}");
        }
        state.m_prev = m_now.or(state.m_prev);
        state.last_meeting_day = Some(game_day);
        let snapshot = state.clone();
        drop(state);

        let writer = Arc::clone(auth);
        if let Err(err) =
            super::auth_db(move || writer.save_pricing_state(&snapshot, meeting.as_ref())).await
        {
            warn!("pricing: failed to persist meeting: {err}");
        }
        self.send_pricing_notice_to_npcs(auth).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_follows_growth_against_a_day_scaled_target() {
        let cfg = PricingConfig::default();
        // 20 days at 0.2%/day targets 4%; 4% growth holds the index.
        assert_eq!(next_index_percent(100, 0.04, 20, &cfg), 100);
        // 24% growth: 0.5 * (0.24 - 0.04) = +10%.
        assert_eq!(next_index_percent(100, 0.24, 20, &cfg), 110);
        // Capped per meeting even on a gold flood.
        assert_eq!(next_index_percent(100, 2.0, 20, &cfg), 110);
        // A 40-day gap targets 8%, so 8% growth is neutral.
        assert_eq!(next_index_percent(100, 0.08, 40, &cfg), 100);
        // Floor and ceiling.
        assert_eq!(next_index_percent(92, -0.5, 20, &cfg), 90);
        assert_eq!(next_index_percent(198, 1.0, 20, &cfg), 200);
    }
}

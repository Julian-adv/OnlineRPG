//! Serin, the swift moon. Mirrors `getMoonPhaseState` in client
//! `celestialSimulation.ts`; its dark evening hosts the merchants' price
//! meeting (doc/PRICING.md).

use crate::world::GameDateTime;

pub const GAME_START_YEAR: i64 = 217;
pub const GAME_DAYS_PER_MONTH: i64 = 30;
pub const GAME_MONTHS_PER_YEAR: i64 = 12;
const SERIN_PERIOD_DAYS: i64 = 20;
const SERIN_PHASE_OFFSET_DAYS: i64 = 5;

/// Days since the calendar epoch (year 217, month 1, day 1).
pub fn game_day_index(datetime: &GameDateTime) -> i64 {
    let year = i64::from(datetime.year).max(GAME_START_YEAR);
    let month = i64::from(datetime.month).clamp(1, GAME_MONTHS_PER_YEAR);
    let day = i64::from(datetime.day).clamp(1, GAME_DAYS_PER_MONTH);
    (year - GAME_START_YEAR) * GAME_DAYS_PER_MONTH * GAME_MONTHS_PER_YEAR
        + (month - 1) * GAME_DAYS_PER_MONTH
        + (day - 1)
}

/// The game day on which Serin stays dark through the evening.
pub fn is_serin_dark_day(game_day: i64) -> bool {
    (game_day + SERIN_PHASE_OFFSET_DAYS).rem_euclid(SERIN_PERIOD_DAYS) == SERIN_PERIOD_DAYS - 1
}

/// Game days until the next dark day (0 = today).
pub fn days_until_serin_dark(game_day: i64) -> i64 {
    (SERIN_PERIOD_DAYS - 1 - (game_day + SERIN_PHASE_OFFSET_DAYS).rem_euclid(SERIN_PERIOD_DAYS))
        .rem_euclid(SERIN_PERIOD_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serin_illumination(game_day: i64, hour: f64) -> f64 {
        let period = SERIN_PERIOD_DAYS as f64;
        let cycle_day = (game_day + SERIN_PHASE_OFFSET_DAYS).rem_euclid(SERIN_PERIOD_DAYS) as f64
            + hour / 24.0
            + 1.0;
        let full = period / 2.0;
        let raw = if cycle_day <= full {
            (cycle_day - 1.0) / (full - 1.0)
        } else {
            1.0 - (cycle_day - full) / (period - full)
        };
        raw.clamp(0.0, 1.0)
    }

    #[test]
    fn dark_day_is_the_evening_with_no_serin_light() {
        let dark = (0..40)
            .filter(|d| is_serin_dark_day(*d))
            .collect::<Vec<_>>();
        assert_eq!(dark, vec![14, 34]);
        // Day 13's evening is a thin sliver; day 14 is dark all day.
        assert_eq!(serin_illumination(14, 20.0), 0.0);
        assert!(serin_illumination(13, 20.0) > 0.0);
        assert!(serin_illumination(15, 20.0) > 0.05);
        assert!((serin_illumination(4, 0.0) - 1.0).abs() < 1e-9);
        assert_eq!(days_until_serin_dark(14), 0);
        assert_eq!(days_until_serin_dark(15), 19);
        assert_eq!(days_until_serin_dark(10), 4);
    }

    #[test]
    fn day_index_counts_from_the_epoch() {
        let dt = |year, month, day| GameDateTime {
            year,
            month,
            day,
            hour: 0,
            minute: 0,
        };
        assert_eq!(game_day_index(&dt(217, 1, 1)), 0);
        assert_eq!(game_day_index(&dt(217, 2, 1)), 30);
        assert_eq!(game_day_index(&dt(218, 1, 15)), 360 + 14);
    }
}

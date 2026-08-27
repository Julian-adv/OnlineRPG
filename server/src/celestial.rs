use crate::types::GameDateTime;
use std::f64::consts::PI;

const DAYS_PER_MONTH: u32 = 30;
const MONTHS_PER_YEAR: u32 = 12;
const DAYS_PER_YEAR: u32 = DAYS_PER_MONTH * MONTHS_PER_YEAR;
const SPRING_EQUINOX_DAY_INDEX: u32 = 90;
const LATITUDE_DEG: f64 = 40.0;
const AXIAL_TILT_DEG: f64 = 24.0;
const HOURS_PER_DAY: f64 = 24.0;
const SERIN_PERIOD_DAYS: i64 = 20;
const SERIN_PHASE_OFFSET_DAYS: i64 = 5;

#[allow(dead_code)]
pub struct SolarDaylightWindow {
    pub sunrise_hour: f64,
    pub sunset_hour: f64,
    pub day_length_hours: f64,
}

fn day_of_year(month: u8, day: u8) -> u32 {
    let clamped_month = (month as u32).clamp(1, MONTHS_PER_YEAR);
    let clamped_day = (day as u32).clamp(1, DAYS_PER_MONTH);
    (clamped_month - 1) * DAYS_PER_MONTH + clamped_day
}

fn get_declination_rad(day_of_year: u32) -> f64 {
    let axial_tilt_rad = AXIAL_TILT_DEG.to_radians();
    let phase =
        (2.0 * PI * (day_of_year as f64 - SPRING_EQUINOX_DAY_INDEX as f64)) / DAYS_PER_YEAR as f64;
    axial_tilt_rad * phase.sin()
}

pub fn get_solar_daylight_window(month: u8, day: u8) -> SolarDaylightWindow {
    let doy = day_of_year(month, day);
    let latitude_rad = LATITUDE_DEG.to_radians();
    let declination = get_declination_rad(doy);
    let cos_hour_angle = -latitude_rad.tan() * declination.tan();

    if cos_hour_angle <= -1.0 {
        return SolarDaylightWindow {
            sunrise_hour: 0.0,
            sunset_hour: HOURS_PER_DAY,
            day_length_hours: HOURS_PER_DAY,
        };
    }

    if cos_hour_angle >= 1.0 {
        return SolarDaylightWindow {
            sunrise_hour: 12.0,
            sunset_hour: 12.0,
            day_length_hours: 0.0,
        };
    }

    let hour_angle = cos_hour_angle.acos();
    let day_length_hours = (HOURS_PER_DAY * hour_angle) / PI;

    SolarDaylightWindow {
        sunrise_hour: 12.0 - day_length_hours / 2.0,
        sunset_hour: 12.0 + day_length_hours / 2.0,
        day_length_hours,
    }
}

fn hour_of_day(datetime: &GameDateTime) -> f64 {
    f64::from(datetime.hour) + f64::from(datetime.minute) / 60.0
}

pub fn is_night(datetime: &GameDateTime) -> bool {
    let window = get_solar_daylight_window(datetime.month, datetime.day);
    hour_of_day(datetime) < window.sunrise_hour || is_after_sunset(datetime)
}

/// Whether the day's night has already begun. Split out from `is_night` so
/// callers that key off the nightfall boundary alone (rather than "is it dark
/// right now") don't re-derive solar time.
pub fn is_after_sunset(datetime: &GameDateTime) -> bool {
    hour_of_day(datetime) >= get_solar_daylight_window(datetime.month, datetime.day).sunset_hour
}

/// Serin (the swift moon) illumination, 0..=1. Mirrors `getMoonPhaseState`
/// in client `celestialSimulation.ts`; proves `is_serin_dark_day`.
#[cfg(test)]
fn serin_illumination(game_day: i64, hour: f64) -> f64 {
    let period = SERIN_PERIOD_DAYS as f64;
    let cycle_day = (game_day + SERIN_PHASE_OFFSET_DAYS).rem_euclid(SERIN_PERIOD_DAYS) as f64
        + hour / HOURS_PER_DAY
        + 1.0;
    let full = period / 2.0;
    let raw = if cycle_day <= full {
        (cycle_day - 1.0) / (full - 1.0)
    } else {
        1.0 - (cycle_day - full) / (period - full)
    };
    raw.clamp(0.0, 1.0)
}

/// The game day on which Serin stays dark through the evening: the
/// merchants' price meeting night (doc/PRICING.md).
pub fn is_serin_dark_day(game_day: i64) -> bool {
    (game_day + SERIN_PHASE_OFFSET_DAYS).rem_euclid(SERIN_PERIOD_DAYS) == SERIN_PERIOD_DAYS - 1
}

#[cfg(test)]
mod serin_tests {
    use super::*;

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
    }
}

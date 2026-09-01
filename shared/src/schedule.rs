//! NPC daily-schedule model, shared by the server (sleep rules, the schedule
//! editor API) and the agent-client (movement, prompts).

use crate::celestial::{get_solar_daylight_window, hour_of_day};
use crate::world::GameDateTime;
use serde::{Deserialize, Serialize};

/// Object type an NPC occupies while asleep.
pub const BED_OBJECT_TYPE: &str = "bed";
const MEAL_DURATION_HOURS: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulePeriod {
    Day,
    Dinner,
    Night,
    Breakfast,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleCondition {
    Day,
    Dinner,
    Night,
    Breakfast,
    Time {
        hour: u32,
        minute: u32,
    },
    TimeRange {
        start_minute: u32,
        end_minute: u32,
    },
    /// Recurring: fires every hour at the given minute (e.g. `"*:00"`).
    Recurring {
        minute: u32,
    },
    /// After sunset on Serin's dark day (doc/PRICING.md).
    Meeting,
}

/// A single schedule entry: go to a position at a specific time condition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduleEntry {
    /// When to activate: a daily period, meeting, exact time, time range, or
    /// hourly recurrence.
    pub at: String,
    /// Target position [x, y, z] (final/rest position).
    pub pos: [f32; 3],
    /// Facing rotation in degrees.
    #[serde(default)]
    pub rotation: f32,
    /// Floor level (0 = ground, 1 = 2nd floor, etc.).
    #[serde(default)]
    pub floor_level: u8,
    /// Human-readable label for LLM prompt context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Object type to interact with after arriving (e.g. "bed").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Object placement ID to interact with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u32>,
    /// Hosts the gathering this entry attends (the guild head at the
    /// price meeting closes it with the decision).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host: bool,
    /// Optional patrol route: waypoints to visit before going to `pos`.
    #[serde(default)]
    pub waypoints: Vec<[f32; 3]>,
    /// Parsed condition (set after deserialization).
    #[serde(skip)]
    pub condition: Option<ScheduleCondition>,
}

impl ScheduleEntry {
    pub fn is_sleeping(&self) -> bool {
        self.action.as_deref() == Some(BED_OBJECT_TYPE)
    }

    pub fn display_label(&self) -> &str {
        self.label.as_deref().unwrap_or("schedule position")
    }

    /// Parse the `at` field into a `ScheduleCondition`. Returns error for invalid formats.
    /// Supports daily periods, exact times, time ranges, hourly recurrence,
    /// and meetings.
    pub fn parse_condition(&mut self) -> Result<(), String> {
        self.condition = Some(match self.at.as_str() {
            "day" => ScheduleCondition::Day,
            "dinner" => ScheduleCondition::Dinner,
            "night" => ScheduleCondition::Night,
            "breakfast" => ScheduleCondition::Breakfast,
            "meeting" => ScheduleCondition::Meeting,
            time_str => {
                if let Some((start, end)) = time_str.split_once('-') {
                    let (start_hour, start_minute) = parse_daily_time(start, time_str)?;
                    let (end_hour, end_minute) = parse_daily_time(end, time_str)?;
                    let start_minute = start_hour * 60 + start_minute;
                    let end_minute = end_hour * 60 + end_minute;
                    if start_minute == end_minute {
                        return Err(format!("empty time range: {time_str}"));
                    }
                    ScheduleCondition::TimeRange {
                        start_minute,
                        end_minute,
                    }
                } else {
                    let (h, m) = time_str
                        .split_once(':')
                        .ok_or_else(|| format!("invalid schedule condition: {time_str}"))?;
                    if h.trim() == "*" {
                        let minute = parse_minute(m, time_str)?;
                        ScheduleCondition::Recurring { minute }
                    } else {
                        let (hour, minute) = parse_daily_time(time_str, time_str)?;
                        ScheduleCondition::Time { hour, minute }
                    }
                }
            }
        });
        Ok(())
    }
}

fn parse_daily_time(value: &str, condition: &str) -> Result<(u32, u32), String> {
    let (hour, minute) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| format!("invalid time range: {condition}"))?;
    let hour = hour
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid hour in: {condition}"))?;
    let minute = parse_minute(minute, condition)?;
    if hour >= 24 {
        return Err(format!("hour out of range in: {condition}"));
    }
    Ok((hour, minute))
}

fn parse_minute(value: &str, condition: &str) -> Result<u32, String> {
    let minute = value
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid minute in: {condition}"))?;
    if minute >= 60 {
        return Err(format!("minute out of range in: {condition}"));
    }
    Ok(minute)
}

pub fn schedule_period(datetime: &GameDateTime) -> SchedulePeriod {
    let window = get_solar_daylight_window(datetime.month, datetime.day);
    let hour = hour_of_day(datetime);

    if hour < window.sunrise_hour - MEAL_DURATION_HOURS {
        SchedulePeriod::Night
    } else if hour < window.sunrise_hour {
        SchedulePeriod::Breakfast
    } else if hour < window.sunset_hour - MEAL_DURATION_HOURS {
        SchedulePeriod::Day
    } else if hour < window.sunset_hour {
        SchedulePeriod::Dinner
    } else {
        SchedulePeriod::Night
    }
}

/// Parse every entry's `at` condition, returning the errors. A failed entry
/// keeps `condition == None` and never activates.
pub fn parse_conditions(entries: &mut [ScheduleEntry]) -> Vec<String> {
    entries
        .iter_mut()
        .filter_map(|entry| entry.parse_condition().err())
        .collect()
}

/// Resolve which schedule entry is currently active based on game time.
/// Before the day's first timed entry, the previous day's last one remains active.
/// Returns `(entry_index, game_hour)` — the hour component ensures recurring
/// entries re-trigger each hour even though the index stays the same.
/// Conditions are pre-validated at load time via `ScheduleEntry::parse_condition`.
pub fn resolve_active_schedule(
    schedule: &[ScheduleEntry],
    period: Option<SchedulePeriod>,
    game_hour: Option<u32>,
    game_minute: Option<u32>,
    is_serin_dark_day: Option<bool>,
) -> (Option<usize>, Option<u32>) {
    let mut best: Option<(u8, usize)> = None;

    for (i, entry) in schedule.iter().enumerate() {
        let condition = match entry.condition.as_ref() {
            Some(c) => c,
            None => continue,
        };
        let matched = match condition {
            ScheduleCondition::Day => {
                matches!(period, Some(SchedulePeriod::Day | SchedulePeriod::Dinner))
            }
            ScheduleCondition::Dinner => period == Some(SchedulePeriod::Dinner),
            ScheduleCondition::Night => matches!(
                period,
                Some(SchedulePeriod::Night | SchedulePeriod::Breakfast)
            ),
            ScheduleCondition::Breakfast => period == Some(SchedulePeriod::Breakfast),
            ScheduleCondition::Time {
                hour: eh,
                minute: em,
            } => match (game_hour, game_minute) {
                (Some(gh), Some(gm)) => gh * 60 + gm >= eh * 60 + em,
                _ => false,
            },
            ScheduleCondition::TimeRange {
                start_minute,
                end_minute,
            } => match (game_hour, game_minute) {
                (Some(hour), Some(minute)) => {
                    let current = hour * 60 + minute;
                    if start_minute < end_minute {
                        current >= *start_minute && current < *end_minute
                    } else {
                        current >= *start_minute || current < *end_minute
                    }
                }
                _ => false,
            },
            ScheduleCondition::Recurring { minute: em } => match (game_hour, game_minute) {
                (Some(_), Some(gm)) => gm >= *em,
                _ => false,
            },
            // Sunset is always between noon and midnight.
            ScheduleCondition::Meeting => {
                is_serin_dark_day == Some(true)
                    && matches!(
                        period,
                        Some(SchedulePeriod::Night | SchedulePeriod::Breakfast)
                    )
                    && game_hour.is_some_and(|h| h >= 12)
            }
        };

        let priority = match condition {
            ScheduleCondition::Meeting => 2,
            ScheduleCondition::Time { .. }
            | ScheduleCondition::TimeRange { .. }
            | ScheduleCondition::Recurring { .. } => 1,
            _ => 0,
        };
        if matched && best.is_none_or(|(best_priority, _)| priority >= best_priority) {
            best = Some((priority, i));
        }
    }

    let mut best = best.map(|(_, i)| i);

    if best.is_none() && game_hour.is_some() && game_minute.is_some() {
        best = schedule
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| match entry.condition.as_ref() {
                Some(ScheduleCondition::Time { hour, minute }) => {
                    Some((hour * 60 + minute, i))
                }
                _ => None,
            })
            .max_by_key(|(minute, _)| *minute)
            .map(|(_, i)| i);
    }

    let hour_for_recurring = best.and_then(|i| {
        if matches!(
            schedule[i].condition,
            Some(ScheduleCondition::Recurring { .. })
        ) {
            game_hour
        } else {
            None
        }
    });
    (best, hour_for_recurring)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: &str) -> ScheduleEntry {
        let mut e = ScheduleEntry {
            at: at.to_string(),
            ..Default::default()
        };
        e.parse_condition().unwrap();
        e
    }

    #[test]
    fn meeting_entry_needs_a_dark_day_evening() {
        let schedule = [entry("day"), entry("night"), entry("meeting")];
        let resolve = |period, hour, dark| {
            resolve_active_schedule(&schedule, Some(period), Some(hour), Some(0), Some(dark)).0
        };
        assert_eq!(resolve(SchedulePeriod::Night, 20, true), Some(2));
        assert_eq!(resolve(SchedulePeriod::Night, 20, false), Some(1));
        assert_eq!(resolve(SchedulePeriod::Day, 15, true), Some(0));
        assert_eq!(resolve(SchedulePeriod::Night, 3, true), Some(1));
        assert_eq!(
            resolve_active_schedule(
                &schedule,
                Some(SchedulePeriod::Night),
                Some(20),
                Some(0),
                None
            )
            .0,
            Some(1)
        );
    }

    #[test]
    fn solar_periods_follow_day_dinner_night_breakfast() {
        let period = |hour, minute| {
            schedule_period(&GameDateTime {
                year: 1,
                month: 3,
                day: 30,
                hour,
                minute,
            })
        };

        assert_eq!(period(6, 0), SchedulePeriod::Day);
        assert_eq!(period(17, 0), SchedulePeriod::Dinner);
        assert_eq!(period(18, 0), SchedulePeriod::Night);
        assert_eq!(period(5, 0), SchedulePeriod::Breakfast);
    }

    #[test]
    fn meal_entries_override_their_parent_periods() {
        let schedule = [
            entry("day"),
            entry("dinner"),
            entry("night"),
            entry("breakfast"),
        ];
        let resolve = |period| {
            resolve_active_schedule(&schedule, Some(period), Some(12), Some(0), None).0
        };

        assert_eq!(resolve(SchedulePeriod::Day), Some(0));
        assert_eq!(resolve(SchedulePeriod::Dinner), Some(1));
        assert_eq!(resolve(SchedulePeriod::Night), Some(2));
        assert_eq!(resolve(SchedulePeriod::Breakfast), Some(3));

        let legacy = [entry("day"), entry("night")];
        assert_eq!(
            resolve_active_schedule(
                &legacy,
                Some(SchedulePeriod::Dinner),
                Some(12),
                Some(0),
                None
            )
            .0,
            Some(0)
        );
        assert_eq!(
            resolve_active_schedule(
                &legacy,
                Some(SchedulePeriod::Breakfast),
                Some(5),
                Some(0),
                None
            )
            .0,
            Some(1)
        );
    }

    #[test]
    fn timed_schedule_wraps_to_the_previous_days_last_entry() {
        let schedule = [entry("4:30"), entry("11:00"), entry("19:00")];
        let resolve = |hour, minute| {
            resolve_active_schedule(&schedule, None, Some(hour), Some(minute), None).0
        };

        assert_eq!(resolve(0, 0), Some(2));
        assert_eq!(resolve(4, 29), Some(2));
        assert_eq!(resolve(4, 30), Some(0));
    }

    #[test]
    fn time_range_overrides_daily_periods_until_its_end() {
        let schedule = [
            entry("3:00-6:00"),
            entry("day"),
            entry("night"),
            entry("breakfast"),
        ];
        let resolve = |period, hour, minute| {
            resolve_active_schedule(
                &schedule,
                Some(period),
                Some(hour),
                Some(minute),
                None,
            )
            .0
        };

        assert_eq!(resolve(SchedulePeriod::Night, 2, 59), Some(2));
        assert_eq!(resolve(SchedulePeriod::Night, 3, 0), Some(0));
        assert_eq!(resolve(SchedulePeriod::Breakfast, 5, 30), Some(0));
        assert_eq!(resolve(SchedulePeriod::Day, 6, 0), Some(1));
    }

    #[test]
    fn time_range_can_cross_midnight() {
        let schedule = [entry("22:00-2:00")];
        let resolve = |hour| {
            resolve_active_schedule(&schedule, None, Some(hour), Some(0), None).0
        };

        assert_eq!(resolve(23), Some(0));
        assert_eq!(resolve(1), Some(0));
        assert_eq!(resolve(2), None);
    }
}

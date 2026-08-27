//! NPC daily-schedule model, shared by the server (sleep rules, the schedule
//! editor API) and the agent-client (movement, prompts).

use serde::{Deserialize, Serialize};

/// Object type an NPC occupies while asleep.
pub const BED_OBJECT_TYPE: &str = "bed";

#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleCondition {
    Day,
    Night,
    Time {
        hour: u32,
        minute: u32,
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
    /// When to activate: "day", "night", "meeting", "H:MM" / "HH:MM", or
    /// "*:MM" (game time).
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
    /// Supports: `"day"`, `"night"`, `"H:MM"` / `"HH:MM"`, or `"*:MM"` (recurring every hour).
    pub fn parse_condition(&mut self) -> Result<(), String> {
        self.condition = Some(match self.at.as_str() {
            "day" => ScheduleCondition::Day,
            "night" => ScheduleCondition::Night,
            "meeting" => ScheduleCondition::Meeting,
            time_str => {
                let (h, m) = time_str
                    .split_once(':')
                    .ok_or_else(|| format!("invalid schedule condition: {time_str}"))?;
                let minute = m
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| format!("invalid minute in: {time_str}"))?;
                if minute >= 60 {
                    return Err(format!("minute out of range in: {time_str}"));
                }
                if h.trim() == "*" {
                    ScheduleCondition::Recurring { minute }
                } else {
                    let hour = h
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| format!("invalid hour in: {time_str}"))?;
                    if hour >= 24 {
                        return Err(format!("hour out of range in: {time_str}"));
                    }
                    ScheduleCondition::Time { hour, minute }
                }
            }
        });
        Ok(())
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
/// Returns `(entry_index, game_hour)` — the hour component ensures recurring
/// entries re-trigger each hour even though the index stays the same.
/// Conditions are pre-validated at load time via `ScheduleEntry::parse_condition`.
pub fn resolve_active_schedule(
    schedule: &[ScheduleEntry],
    is_night: Option<bool>,
    game_hour: Option<u32>,
    game_minute: Option<u32>,
    is_serin_dark_day: Option<bool>,
) -> (Option<usize>, Option<u32>) {
    let mut best: Option<usize> = None;

    for (i, entry) in schedule.iter().enumerate() {
        let condition = match entry.condition.as_ref() {
            Some(c) => c,
            None => continue,
        };
        let matched = match condition {
            ScheduleCondition::Day => is_night == Some(false),
            ScheduleCondition::Night => is_night == Some(true),
            ScheduleCondition::Time {
                hour: eh,
                minute: em,
            } => match (game_hour, game_minute) {
                (Some(gh), Some(gm)) => gh * 60 + gm >= eh * 60 + em,
                _ => false,
            },
            ScheduleCondition::Recurring { minute: em } => match (game_hour, game_minute) {
                (Some(_), Some(gm)) => gm >= *em,
                _ => false,
            },
            // Sunset is always between noon and midnight.
            ScheduleCondition::Meeting => {
                is_serin_dark_day == Some(true)
                    && is_night == Some(true)
                    && game_hour.is_some_and(|h| h >= 12)
            }
        };

        if matched {
            best = Some(i);
        }
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
        let resolve = |night, hour, dark| {
            resolve_active_schedule(&schedule, Some(night), Some(hour), Some(0), Some(dark)).0
        };
        assert_eq!(resolve(true, 20, true), Some(2));
        assert_eq!(resolve(true, 20, false), Some(1));
        assert_eq!(resolve(false, 15, true), Some(0));
        assert_eq!(resolve(true, 3, true), Some(1));
        assert_eq!(
            resolve_active_schedule(&schedule, Some(true), Some(20), Some(0), None).0,
            Some(1)
        );
    }
}

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
}

/// A single schedule entry: go to a position at a specific time condition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduleEntry {
    /// When to activate: "day", "night", "H:MM" / "HH:MM", or "*:MM" (game time).
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

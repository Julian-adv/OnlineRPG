//! Recently-met players: whenever two players come into each other's AOI
//! (join or movement fan-out), both remember the meeting. In-memory only,
//! per session, capped per player; the client polls the list on demand.

use std::collections::VecDeque;

use onlinerpg_shared::messages::EncounterEntry;

use super::GameState;
use crate::types::{Player, PlayerId, ServerMessage};

/// Most meetings remembered per player; the oldest falls off first.
pub(crate) const ENCOUNTER_CAP: usize = 100;

/// Re-meeting the same player within this window refreshes `last_met` but
/// does not count as a new meeting — AOI flicker in a crowd is not "met
/// them 50 times".
pub(crate) const ENCOUNTER_DEDUP_SECS: i64 = 600;

/// Record one seen player into an owner's queue: refresh and move to the
/// back if known, append (evicting the oldest over cap) if new.
pub(crate) fn note_encounter(
    q: &mut VecDeque<EncounterEntry>,
    character_id: i64,
    name: &str,
    level: u32,
    now_unix: i64,
) {
    if let Some(pos) = q.iter().position(|e| e.character_id == character_id) {
        let Some(mut entry) = q.remove(pos) else {
            return;
        };
        if now_unix - entry.last_met_unix >= ENCOUNTER_DEDUP_SECS {
            entry.met_count += 1;
        }
        entry.last_met_unix = now_unix;
        entry.name = name.to_string();
        entry.level = level;
        q.push_back(entry);
    } else {
        if q.len() >= ENCOUNTER_CAP {
            q.pop_front();
        }
        q.push_back(EncounterEntry {
            character_id,
            name: name.to_string(),
            level,
            last_met_unix: now_unix,
            met_count: 1,
        });
    }
}

impl GameState {
    /// `me` and each of `others` just came into sight of one another: record
    /// the meeting on both sides. Official NPCs neither remember nor are
    /// remembered. Callers on the move path only call with a non-empty list,
    /// so the idle-move majority never touches these locks.
    pub(super) async fn record_encounters(&self, me: &Player, others: &[Player]) {
        if me.is_official_npc || others.is_empty() {
            return;
        }
        let now = crate::auth::unix_now();
        let characters = self.player_characters.read().await;
        let character_id_of = |id: &PlayerId| characters.get(id).map(|(cid, _, _)| *cid);
        let my_character_id = character_id_of(&me.id);
        let mut map = self.recent_encounters.write().await;
        for other in others {
            if other.is_official_npc {
                continue;
            }
            if let Some(cid) = character_id_of(&other.id) {
                note_encounter(
                    map.entry(me.id).or_default(),
                    cid,
                    &other.name,
                    other.level,
                    now,
                );
            }
            if let Some(cid) = my_character_id {
                note_encounter(
                    map.entry(other.id).or_default(),
                    cid,
                    &me.name,
                    me.level,
                    now,
                );
            }
        }
    }

    /// Answer `ClientMessage::RequestRecentEncounters`: newest first.
    pub async fn send_recent_encounters(&self, player_id: &PlayerId) {
        let entries: Vec<EncounterEntry> = self
            .recent_encounters
            .read()
            .await
            .get(player_id)
            .map(|q| q.iter().rev().cloned().collect())
            .unwrap_or_default();
        self.send_direct_message(player_id, ServerMessage::RecentEncounters { entries })
            .await;
    }

    pub(super) async fn remove_player_encounters(&self, player_id: &PlayerId) {
        self.recent_encounters.write().await.remove(player_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(q: &mut VecDeque<EncounterEntry>, cid: i64, now: i64) {
        note_encounter(q, cid, &format!("p{cid}"), 1, now);
    }

    #[test]
    fn new_meetings_append_and_evict_the_oldest_over_cap() {
        let mut q = VecDeque::new();
        for cid in 0..(ENCOUNTER_CAP as i64 + 5) {
            note(&mut q, cid, cid);
        }
        assert_eq!(q.len(), ENCOUNTER_CAP);
        assert_eq!(q.front().unwrap().character_id, 5, "oldest evicted first");
        assert_eq!(q.back().unwrap().character_id, ENCOUNTER_CAP as i64 + 4);
    }

    #[test]
    fn a_quick_re_meeting_refreshes_without_counting() {
        let mut q = VecDeque::new();
        note(&mut q, 1, 0);
        note(&mut q, 2, 1);
        note(&mut q, 1, ENCOUNTER_DEDUP_SECS - 1);
        assert_eq!(q.len(), 2);
        let e = q.back().unwrap();
        assert_eq!(e.character_id, 1, "refreshed entry moves to the back");
        assert_eq!(e.met_count, 1, "AOI flicker is not a new meeting");
        assert_eq!(e.last_met_unix, ENCOUNTER_DEDUP_SECS - 1);
    }

    #[test]
    fn a_later_re_meeting_counts_and_updates_the_snapshot() {
        let mut q = VecDeque::new();
        note_encounter(&mut q, 1, "old_name", 3, 0);
        note_encounter(&mut q, 1, "new_name", 4, ENCOUNTER_DEDUP_SECS);
        assert_eq!(q.len(), 1);
        let e = q.back().unwrap();
        assert_eq!(e.met_count, 2);
        assert_eq!(e.name, "new_name");
        assert_eq!(e.level, 4);
    }
}

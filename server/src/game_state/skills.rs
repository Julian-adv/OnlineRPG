//! Trained-skill state: the per-player `Skills` map, its dirty tracking, and
//! XP grants. Mirrors the gold/inventory pattern — skills live outside the
//! broadcast `Player` struct (private to their owner), are registered on
//! EnterGame, flushed through the same dirty-set saves, and detached on
//! logout. Nothing grants skill XP yet; the first caller is the fishing
//! system (doc/FISHING.md).

use onlinerpg_shared::skills::{SkillId, SkillXpResult, Skills};
use tracing::warn;

use super::GameState;
use crate::auth::SkillRow;
use crate::types::{PlayerId, ServerMessage};

/// Convert DB rows to the in-memory map. Unknown skill ids (rows written by
/// a newer server) are skipped here but survive on disk: saves go through an
/// upsert that never deletes rows.
pub(crate) fn skills_from_rows(rows: &[SkillRow]) -> Skills {
    let mut skills = Skills::default();
    for row in rows {
        match row.skill_id.parse::<SkillId>() {
            Ok(id) => {
                skills.map.insert(
                    id,
                    onlinerpg_shared::skills::SkillProgress {
                        level: row.level,
                        xp: row.xp,
                    },
                );
            }
            Err(()) => warn!(
                "Ignoring unknown skill id '{}' (newer server?)",
                row.skill_id
            ),
        }
    }
    skills
}

fn skills_to_rows(skills: &Skills) -> Vec<SkillRow> {
    let mut rows: Vec<SkillRow> = skills
        .map
        .iter()
        .map(|(id, progress)| SkillRow {
            skill_id: id.as_str().to_string(),
            level: progress.level,
            xp: progress.xp,
        })
        .collect();
    rows.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));
    rows
}

impl GameState {
    pub async fn register_player_skills(&self, player_id: &PlayerId, skills: Skills) {
        let mut map = self.player_skills.write().await;
        map.insert(*player_id, skills);
    }

    /// One skill's level, without cloning the whole map entry.
    pub async fn skill_level(&self, player_id: &PlayerId, skill: SkillId) -> u32 {
        let map = self.player_skills.read().await;
        map.get(player_id).map_or(0, |s| s.get(skill).level)
    }

    /// Grant skill XP: updates the map, marks the player dirty for the next
    /// periodic save, and tells the owner via `SkillXpGained`. Returns what
    /// `Skills::add_xp` reported (`None` = capped, nothing happened).
    pub async fn add_skill_xp(
        &self,
        player_id: &PlayerId,
        skill: SkillId,
        amount: u64,
    ) -> Option<SkillXpResult> {
        let result = {
            let mut map = self.player_skills.write().await;
            map.get_mut(player_id)?.add_xp(skill, amount)?
        };
        {
            let mut dirty = self.dirty_skills.write().await;
            dirty.insert(*player_id);
        }
        self.send_direct_message(
            player_id,
            ServerMessage::SkillXpGained {
                skill,
                xp_amount: result.xp_amount,
                total_xp: result.total_xp,
                new_level: result.new_level,
                leveled_up: result.leveled_up,
            },
        )
        .await;
        Some(result)
    }

    /// Snapshot a player's skills as save rows and drop the in-memory entry.
    /// The logout twin of `take_player_inventory`.
    pub(super) async fn take_player_skills(
        &self,
        player_id: &PlayerId,
    ) -> Option<(i64, Vec<SkillRow>)> {
        let character_id = {
            let characters = self.player_characters.read().await;
            characters.get(player_id).map(|(id, _, _)| *id)?
        };
        {
            let mut dirty = self.dirty_skills.write().await;
            dirty.remove(player_id);
        }
        let skills = {
            let mut map = self.player_skills.write().await;
            map.remove(player_id)?
        };
        Some((character_id, skills_to_rows(&skills)))
    }

    /// Save rows for every player marked dirty since the last flush, with
    /// the drained ids so a failed save can re-mark them (same retry contract
    /// as `collect_dirty_character_states`).
    pub(super) async fn collect_dirty_skill_states(
        &self,
    ) -> (Vec<PlayerId>, Vec<(i64, Vec<SkillRow>)>) {
        let dirty: Vec<PlayerId> = {
            let mut dirty = self.dirty_skills.write().await;
            dirty.drain().collect()
        };
        if dirty.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let characters = self.player_characters.read().await;
        let skills_map = self.player_skills.read().await;
        let mut rows = Vec::with_capacity(dirty.len());
        for player_id in &dirty {
            let Some((character_id, _, _)) = characters.get(player_id) else {
                continue;
            };
            let Some(skills) = skills_map.get(player_id) else {
                continue;
            };
            rows.push((*character_id, skills_to_rows(skills)));
        }
        rows.sort_by_key(|(character_id, _)| *character_id);
        (dirty, rows)
    }

    /// Put drained skill dirtiness back after a failed batch save so the next
    /// flush retries it.
    pub(super) async fn restore_dirty_skills(&self, ids: Vec<PlayerId>) {
        if !ids.is_empty() {
            self.dirty_skills.write().await.extend(ids);
        }
    }

    /// Save rows for every connected player (shutdown snapshot).
    pub(super) async fn collect_all_skill_states(&self) -> Vec<(i64, Vec<SkillRow>)> {
        let characters = self.player_characters.read().await;
        let skills_map = self.player_skills.read().await;
        let mut rows = Vec::with_capacity(skills_map.len());
        for (player_id, skills) in skills_map.iter() {
            let Some((character_id, _, _)) = characters.get(player_id) else {
                continue;
            };
            rows.push((*character_id, skills_to_rows(skills)));
        }
        rows.sort_by_key(|(character_id, _)| *character_id);
        rows
    }

    /// Drop skill state for a player that is being removed without a persist
    /// (the take/persist paths already removed it in the normal case).
    pub(super) async fn forget_player_skills(&self, player_id: &PlayerId) {
        self.player_skills.write().await.remove(player_id);
        self.dirty_skills.write().await.remove(player_id);
    }
}

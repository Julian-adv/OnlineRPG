//! Titles (doc/TITLES.md): the per-boss damage log and its grant on death,
//! the fishing-catch grant, and which earned title a player shows
//! (`Player.title`).

use std::collections::HashMap;

use onlinerpg_shared::ServerMessage;
use tracing::{info, warn};

use crate::auth::AuthService;
use crate::title_defs;
use crate::types::PlayerId;

use super::GameState;

/// Percent of a boss's total damage that earns the shared kill title.
pub(super) const TITLE_DAMAGE_SHARE: u64 = 50;
/// Percent that earns the solo title (100 would let a passer-by's one hit
/// spoil it).
pub(super) const TITLE_SOLO_SHARE: u64 = 90;

/// Characters that earned a title from this damage log, with whether the
/// solo tier applies. Integer math on percents so ties land on the fence.
pub(super) fn qualifying(damage: &HashMap<i64, u64>) -> Vec<(i64, bool)> {
    let total: u64 = damage.values().sum();
    if total == 0 {
        return Vec::new();
    }
    let mut out: Vec<(i64, bool)> = damage
        .iter()
        .filter(|(_, &d)| d * 100 >= total * TITLE_DAMAGE_SHARE)
        .map(|(&id, &d)| (id, d * 100 >= total * TITLE_SOLO_SHARE))
        .collect();
    out.sort_unstable();
    out
}

impl GameState {
    /// Log damage a player dealt to a dungeon boss. Callers gate on the
    /// floor, so surface combat never reaches the dungeon index.
    pub(super) async fn record_boss_damage(
        &self,
        monster_id: &str,
        player_id: &PlayerId,
        damage: u32,
    ) {
        if damage == 0 {
            return;
        }
        let is_boss = self
            .dungeon_monsters
            .read()
            .await
            .get(monster_id)
            .is_some_and(|m| m.is_boss);
        if !is_boss {
            return;
        }
        // Official NPCs are scenery, not contenders.
        let official = self
            .players
            .read()
            .await
            .get(player_id)
            .is_none_or(|p| p.is_official_npc);
        if official {
            return;
        }
        let Some(character_id) = self.character_id_of(player_id).await else {
            return;
        };
        let mut log = self.boss_damage.write().await;
        *log.entry(monster_id.to_string())
            .or_default()
            .entry(character_id)
            .or_insert(0) += u64::from(damage);
    }

    pub(super) async fn clear_boss_damage(&self) {
        self.boss_damage.write().await.clear();
    }

    /// Settle a dead boss's damage log: grant titles, then forget the log.
    /// `auth` is None only in tests, where the grant stays in memory.
    pub(super) async fn grant_boss_kill_titles(
        &self,
        monster_id: &str,
        boss_type: &str,
        auth: Option<&AuthService>,
    ) {
        let Some(log) = self.boss_damage.write().await.remove(monster_id) else {
            return;
        };
        let (shared, solo) = title_defs::boss_kill_titles(boss_type);
        for (character_id, is_solo) in qualifying(&log) {
            if let Some(def) = shared {
                self.grant_title(character_id, &def.id, auth).await;
            }
            if let Some(def) = solo.filter(|_| is_solo) {
                self.grant_title(character_id, &def.id, auth).await;
            }
        }
    }

    /// A landed catch of a title-worthy fish. Official NPCs are scenery,
    /// not contenders, here as on the boss log.
    pub(super) async fn grant_fishing_catch_title(
        &self,
        player_id: &PlayerId,
        item_def_id: &str,
        auth: Option<&AuthService>,
    ) {
        let Some(def) = title_defs::fishing_catch_title(item_def_id) else {
            return;
        };
        let official = self
            .players
            .read()
            .await
            .get(player_id)
            .is_none_or(|p| p.is_official_npc);
        if official {
            return;
        }
        let Some(character_id) = self.character_id_of(player_id).await else {
            return;
        };
        self.grant_title(character_id, &def.id, auth).await;
    }

    async fn grant_title(&self, character_id: i64, title: &str, auth: Option<&AuthService>) {
        let persisted = match auth {
            Some(auth) => {
                let auth = auth.clone();
                let owned = title.to_string();
                match super::auth_db(move || auth.grant_title(character_id, &owned)).await {
                    Ok(inserted) => inserted,
                    Err(err) => {
                        warn!("Failed to grant title '{title}' to character {character_id}: {err}");
                        return;
                    }
                }
            }
            None => true,
        };
        let Some(player_id) = self.player_id_of_character(character_id).await else {
            if persisted {
                info!("Character {character_id} earned title '{title}' while offline");
            }
            return;
        };
        let first = {
            let mut titles = self.player_titles.write().await;
            let list = titles.entry(player_id).or_default();
            if list.iter().any(|t| t == title) {
                return;
            }
            let first = list.is_empty();
            list.push(title.to_string());
            title_defs::sort_ids(list);
            first
        };
        info!(
            "Player {} earned title '{title}'",
            self.player_name_of(&player_id).await
        );
        self.send_direct_message(
            &player_id,
            ServerMessage::TitleEarned {
                title: title.to_string(),
            },
        )
        .await;
        // Auto-show only a first-ever title, or one that supersedes the shown
        // one; any other pick (an explicit "none" included) is the player's.
        let shown = self.shown_title(&player_id).await;
        let promote = match shown.as_deref() {
            None => first,
            Some(current) => {
                title_defs::title_def(title).and_then(|d| d.supersedes.as_deref()) == Some(current)
            }
        };
        if promote {
            self.set_active_title(&player_id, Some(title.to_string()), auth)
                .await;
        } else {
            self.send_player_titles(&player_id).await;
        }
    }

    async fn player_id_of_character(&self, character_id: i64) -> Option<PlayerId> {
        let characters = self.player_characters.read().await;
        characters
            .iter()
            .find(|(_, (id, _, _))| *id == character_id)
            .map(|(pid, _)| *pid)
    }

    async fn shown_title(&self, player_id: &PlayerId) -> Option<String> {
        self.players
            .read()
            .await
            .get(player_id)
            .and_then(|p| p.title.clone())
    }

    /// Seed a player's earned titles at entry (`Player.title` is set by the
    /// caller from the same record).
    pub async fn set_player_titles(&self, player_id: &PlayerId, mut titles: Vec<String>) {
        title_defs::sort_ids(&mut titles);
        self.player_titles.write().await.insert(*player_id, titles);
    }

    pub async fn remove_player_titles(&self, player_id: &PlayerId) {
        self.player_titles.write().await.remove(player_id);
    }

    pub async fn send_player_titles(&self, player_id: &PlayerId) {
        let titles = self
            .player_titles
            .read()
            .await
            .get(player_id)
            .cloned()
            .unwrap_or_default();
        let active = self.shown_title(player_id).await;
        self.send_direct_message(player_id, ServerMessage::PlayerTitles { titles, active })
            .await;
    }

    /// Show `title` (an earned one) or nothing. Unknown or unearned ids are
    /// ignored — the client only offers earned ones.
    pub async fn set_active_title(
        &self,
        player_id: &PlayerId,
        title: Option<String>,
        auth: Option<&AuthService>,
    ) {
        let Some(character_id) = self.character_id_of(player_id).await else {
            return;
        };
        if let Some(t) = &title {
            let earned = self
                .player_titles
                .read()
                .await
                .get(player_id)
                .is_some_and(|list| list.contains(t));
            if !earned {
                warn!(
                    "Player {} asked to show unearned title '{t}'",
                    self.player_name_of(player_id).await
                );
                return;
            }
        }
        if let Some(auth) = auth {
            let auth = auth.clone();
            let persisted = title.clone();
            if let Err(err) =
                super::auth_db(move || auth.set_active_title(character_id, persisted.as_deref()))
                    .await
            {
                warn!("Failed to save active title for character {character_id}: {err}");
                return;
            }
        }
        let at = {
            let mut players = self.players.write().await;
            let Some(p) = players.get_mut(player_id) else {
                return;
            };
            p.title = title.clone();
            (p.position, p.floor_level)
        };
        // The owner is inside their own radius, so this reaches them too.
        self.send_direct_message_to_players_within_position(
            &at.0,
            at.1,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::PlayerTitleChanged {
                player_id: *player_id,
                title,
            },
            None,
        )
        .await;
    }

    /// `/title` — list, `/title N` to show the Nth, `/title off` for none.
    pub async fn handle_title_command(
        &self,
        player_id: &PlayerId,
        args: &str,
        auth: Option<&AuthService>,
    ) {
        let titles = self
            .player_titles
            .read()
            .await
            .get(player_id)
            .cloned()
            .unwrap_or_default();
        if args.is_empty() {
            if titles.is_empty() {
                self.send_system_message(player_id, "You have no titles yet.")
                    .await;
                return;
            }
            let shown = self.shown_title(player_id).await;
            let mut lines = vec!["Titles (/title N to show, /title off to hide):".to_string()];
            for (i, id) in titles.iter().enumerate() {
                let name = title_defs::title_def(id).map_or(id.as_str(), |d| d.name.as_str());
                let mark = if shown.as_deref() == Some(id) {
                    "*"
                } else {
                    " "
                };
                lines.push(format!("{mark}{}. {name}", i + 1));
            }
            self.send_system_message(player_id, lines.join("\n")).await;
            return;
        }
        if args.eq_ignore_ascii_case("off") || args.eq_ignore_ascii_case("none") {
            self.set_active_title(player_id, None, auth).await;
            return;
        }
        let picked = args
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| titles.get(i).cloned());
        match picked {
            Some(id) => self.set_active_title(player_id, Some(id), auth).await,
            None => {
                self.send_system_message(player_id, "Usage: /title, /title N, /title off")
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(entries: &[(i64, u64)]) -> HashMap<i64, u64> {
        entries.iter().copied().collect()
    }

    #[test]
    fn half_share_earns_the_title_and_ninety_percent_the_solo_tier() {
        assert_eq!(qualifying(&log(&[(1, 100)])), vec![(1, true)]);
        assert_eq!(qualifying(&log(&[(1, 90), (2, 10)])), vec![(1, true)]);
        assert_eq!(qualifying(&log(&[(1, 89), (2, 11)])), vec![(1, false)]);
        assert_eq!(
            qualifying(&log(&[(1, 50), (2, 50)])),
            vec![(1, false), (2, false)]
        );
        assert_eq!(qualifying(&log(&[(1, 49), (2, 51)])), vec![(2, false)]);
        assert!(qualifying(&log(&[(1, 30), (2, 30), (3, 40)])).is_empty());
        assert!(qualifying(&log(&[])).is_empty());
    }
}

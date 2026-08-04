use crate::types::{AttackRejectReason, ClientKind};
use onlinerpg_shared::inventory::ArmorConstruction;
use onlinerpg_shared::skills::{armor_skill_construction, skill_xp_for_level, SkillId};
use onlinerpg_shared::PhysicalDamageResult;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct AttackStats {
    pub attacks: u64,
    pub hits: u64,
    pub kills: u64,
    pub xp: u64,
}

impl AttackStats {
    fn record(&mut self, hit: bool, kill: bool, xp: u64) {
        self.attacks = self.attacks.saturating_add(1);
        self.hits = self.hits.saturating_add(u64::from(hit));
        self.kills = self.kills.saturating_add(u64::from(kill));
        self.xp = self.xp.saturating_add(xp);
    }

    fn hit_rate(self) -> f64 {
        if self.attacks == 0 {
            0.0
        } else {
            self.hits as f64 * 100.0 / self.attacks as f64
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DefenseStats {
    pub defenses: u64,
    pub hits_taken: u64,
    pub avoids: u64,
    pub xp: u64,
}

impl DefenseStats {
    fn record(&mut self, monster_hit: bool, xp: u64) {
        self.defenses = self.defenses.saturating_add(1);
        self.hits_taken = self.hits_taken.saturating_add(u64::from(monster_hit));
        self.avoids = self.avoids.saturating_add(u64::from(!monster_hit));
        self.xp = self.xp.saturating_add(xp);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct HealingStats {
    pub uses: u64,
    pub restored_hp: u64,
    pub xp: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct MitigationStats {
    pub hits: u64,
    pub raw_damage: u64,
    pub mitigated_damage: u64,
    pub final_damage: u64,
}

impl MitigationStats {
    fn record(&mut self, result: PhysicalDamageResult) {
        if result.raw_damage == 0 {
            return;
        }
        self.hits = self.hits.saturating_add(1);
        self.raw_damage = self.raw_damage.saturating_add(u64::from(result.raw_damage));
        self.mitigated_damage = self
            .mitigated_damage
            .saturating_add(u64::from(result.mitigated_damage));
        self.final_damage = self
            .final_damage
            .saturating_add(u64::from(result.final_damage));
    }
}

impl HealingStats {
    fn record(&mut self, restored_hp: u32, xp: u64) {
        self.uses = self.uses.saturating_add(1);
        self.restored_hp = self.restored_hp.saturating_add(u64::from(restored_hp));
        self.xp = self.xp.saturating_add(xp);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RejectionStats {
    pub invalid_target: u64,
    pub out_of_range: u64,
    pub attacker_dead: u64,
    pub not_in_game: u64,
    pub cooldown: u64,
}

impl RejectionStats {
    fn record(&mut self, reason: AttackRejectReason) {
        let counter = match reason {
            AttackRejectReason::InvalidTarget => &mut self.invalid_target,
            AttackRejectReason::OutOfRange => &mut self.out_of_range,
            AttackRejectReason::AttackerDead => &mut self.attacker_dead,
            AttackRejectReason::NotInGame => &mut self.not_in_game,
            AttackRejectReason::Cooldown => &mut self.cooldown,
        };
        *counter = counter.saturating_add(1);
    }

    fn total(self) -> u64 {
        self.invalid_target
            .saturating_add(self.out_of_range)
            .saturating_add(self.attacker_dead)
            .saturating_add(self.not_in_game)
            .saturating_add(self.cooldown)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SkillSaveStats {
    pub periodic_batches: u64,
    pub logout_batches: u64,
    pub shutdown_batches: u64,
    pub rows_written: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SkillSaveKind {
    Periodic,
    Logout,
    Shutdown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MetricsData {
    attack_requests: u64,
    resolved_attacks: u64,
    rejections: RejectionStats,
    weapon: AttackStats,
    weapon_by_skill: BTreeMap<String, AttackStats>,
    weapon_by_skill_band: [AttackStats; 4],
    weapon_by_difficulty: [AttackStats; 3],
    weapon_by_client: [AttackStats; 3],
    weapon_by_level_pair: BTreeMap<(u32, usize), AttackStats>,
    weapon_by_monster: BTreeMap<String, AttackStats>,
    cadence_samples: u64,
    cadence_total_ms: u64,
    cadence_min_ms: Option<u64>,
    cadence_max_ms: Option<u64>,
    weapon_xp_messages: u64,
    weapon_rows_created: u64,
    defense: DefenseStats,
    defense_by_skill: BTreeMap<String, DefenseStats>,
    defense_by_skill_band: [DefenseStats; 4],
    defense_xp_messages: u64,
    defense_rows_created: u64,
    mitigation: MitigationStats,
    mitigation_by_type: BTreeMap<String, MitigationStats>,
    mitigation_by_construction: BTreeMap<String, MitigationStats>,
    healing: HealingStats,
    healing_by_skill_band: [HealingStats; 4],
    healing_xp_messages: u64,
    healing_rows_created: u64,
    saves: SkillSaveStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkillBalanceSnapshot {
    pub uptime_secs: u64,
    pub attack_requests: u64,
    pub resolved_attacks: u64,
    pub rejections: RejectionStats,
    pub weapon: AttackStats,
    pub weapon_by_skill: BTreeMap<String, AttackStats>,
    pub weapon_by_skill_band: [AttackStats; 4],
    pub weapon_by_difficulty: [AttackStats; 3],
    pub weapon_by_client: [AttackStats; 3],
    pub weapon_by_level_pair: BTreeMap<(u32, usize), AttackStats>,
    pub weapon_by_monster: BTreeMap<String, AttackStats>,
    pub cadence_samples: u64,
    pub cadence_total_ms: u64,
    pub cadence_min_ms: Option<u64>,
    pub cadence_max_ms: Option<u64>,
    pub weapon_xp_messages: u64,
    pub weapon_rows_created: u64,
    pub defense: DefenseStats,
    pub defense_by_skill: BTreeMap<String, DefenseStats>,
    pub defense_by_skill_band: [DefenseStats; 4],
    pub defense_xp_messages: u64,
    pub defense_rows_created: u64,
    pub mitigation: MitigationStats,
    pub mitigation_by_type: BTreeMap<String, MitigationStats>,
    pub mitigation_by_construction: BTreeMap<String, MitigationStats>,
    pub healing: HealingStats,
    pub healing_by_skill_band: [HealingStats; 4],
    pub healing_xp_messages: u64,
    pub healing_rows_created: u64,
    pub saves: SkillSaveStats,
}

impl SkillBalanceSnapshot {
    fn average_cadence_ms(&self, fallback: Duration) -> u64 {
        self.cadence_total_ms
            .checked_div(self.cadence_samples)
            .unwrap_or(fallback.as_millis() as u64)
    }

    fn projection(&self, level: u32, cadence: Duration) -> String {
        if self.weapon.attacks == 0 || self.weapon.xp == 0 {
            return format!("lv{level}=n/a");
        }
        let target_xp = skill_xp_for_level(level);
        let attacks = target_xp
            .saturating_mul(self.weapon.attacks)
            .div_ceil(self.weapon.xp);
        let seconds = attacks
            .saturating_mul(self.average_cadence_ms(cadence))
            .div_ceil(1_000);
        format!("lv{level}={attacks}attacks/{}", format_duration(seconds))
    }

    pub(super) fn render(&self, cadence: Duration) -> String {
        let skill_bands = ["0-4", "5-14", "15-24", "25-30"]
            .into_iter()
            .zip(self.weapon_by_skill_band)
            .map(|(label, stats)| format!("{label}:{}@{:.1}%", stats.attacks, stats.hit_rate()))
            .collect::<Vec<_>>()
            .join(",");
        let difficulty = ["weak", "peer", "strong"]
            .into_iter()
            .zip(self.weapon_by_difficulty)
            .map(|(label, stats)| format!("{label}:{}xp/{}a", stats.xp, stats.attacks))
            .collect::<Vec<_>>()
            .join(",");
        let clients = ["web", "cli", "other"]
            .into_iter()
            .zip(self.weapon_by_client)
            .map(|(label, stats)| format!("{label}:{}", stats.attacks))
            .collect::<Vec<_>>()
            .join(",");
        let level_pairs = self
            .weapon_by_level_pair
            .iter()
            .map(|((character_level, skill_band), stats)| {
                let skill_label = ["0-4", "5-14", "15-24", "25-30"][*skill_band];
                format!(
                    "c{character_level}/s{skill_label}:{}@{:.1}%",
                    stats.attacks,
                    stats.hit_rate()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let monsters = self
            .weapon_by_monster
            .iter()
            .map(|(name, stats)| format!("{name}:{}xp/{}a", stats.xp, stats.attacks))
            .collect::<Vec<_>>()
            .join(",");
        let skills = self
            .weapon_by_skill
            .iter()
            .map(|(skill, stats)| {
                format!(
                    "{skill}:{}@{:.1}%/{}xp",
                    stats.attacks,
                    stats.hit_rate(),
                    stats.xp
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let projections = [5, 15, 25, 30]
            .map(|level| self.projection(level, cadence))
            .join(",");
        let defense_skills = self
            .defense_by_skill
            .iter()
            .map(|(skill, stats)| {
                format!(
                    "{skill}:{}d/{}avoid/{}xp",
                    stats.defenses, stats.avoids, stats.xp
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let defense_bands = ["0-4", "5-14", "15-24", "25-30"]
            .into_iter()
            .zip(self.defense_by_skill_band)
            .map(|(label, stats)| format!("{label}:{}d/{}avoid", stats.defenses, stats.avoids))
            .collect::<Vec<_>>()
            .join(",");
        let healing_bands = ["0-4", "5-14", "15-24", "25-30"]
            .into_iter()
            .zip(self.healing_by_skill_band)
            .map(|(label, stats)| {
                format!(
                    "{label}:{}u/{}hp/{}xp",
                    stats.uses, stats.restored_hp, stats.xp
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let mitigation_types = self
            .mitigation_by_type
            .iter()
            .map(|(damage_type, stats)| {
                format!(
                    "{damage_type}:{}raw/{}mit/{}final",
                    stats.raw_damage, stats.mitigated_damage, stats.final_damage
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let mitigation_constructions = self
            .mitigation_by_construction
            .iter()
            .map(|(construction, stats)| {
                format!(
                    "{construction}:{}raw/{}mit/{}final",
                    stats.raw_damage, stats.mitigated_damage, stats.final_damage
                )
            })
            .collect::<Vec<_>>()
            .join(",");

        format!(
            "skill_balance uptime={} requests={} resolved={} rejected={} cooldown={} weapon={} hits={} hit_rate={:.1}% kills={} xp={} cadence_ms={}/{:?}/{:?} skills=[{}] bands=[{}] difficulty=[{}] clients=[{}] levels=[{}] monsters=[{}] messages={} new_rows={} defense={} hits_taken={} avoids={} defense_xp={} defense_skills=[{}] defense_bands=[{}] defense_messages={} defense_new_rows={} mitigation_hits={} raw_damage={} mitigated_damage={} final_damage={} mitigation_types=[{}] mitigation_constructions=[{}] bandage_uses={} restored_hp={} healing_xp={} healing_bands=[{}] healing_messages={} healing_new_rows={} saves={}/{}/{} rows_written={} projections=[{}]",
            format_duration(self.uptime_secs),
            self.attack_requests,
            self.resolved_attacks,
            self.rejections.total(),
            self.rejections.cooldown,
            self.weapon.attacks,
            self.weapon.hits,
            self.weapon.hit_rate(),
            self.weapon.kills,
            self.weapon.xp,
            self.average_cadence_ms(cadence),
            self.cadence_min_ms,
            self.cadence_max_ms,
            skills,
            skill_bands,
            difficulty,
            clients,
            level_pairs,
            monsters,
            self.weapon_xp_messages,
            self.weapon_rows_created,
            self.defense.defenses,
            self.defense.hits_taken,
            self.defense.avoids,
            self.defense.xp,
            defense_skills,
            defense_bands,
            self.defense_xp_messages,
            self.defense_rows_created,
            self.mitigation.hits,
            self.mitigation.raw_damage,
            self.mitigation.mitigated_damage,
            self.mitigation.final_damage,
            mitigation_types,
            mitigation_constructions,
            self.healing.uses,
            self.healing.restored_hp,
            self.healing.xp,
            healing_bands,
            self.healing_xp_messages,
            self.healing_rows_created,
            self.saves.periodic_batches,
            self.saves.logout_batches,
            self.saves.shutdown_batches,
            self.saves.rows_written,
            projections,
        )
    }
}

pub(super) struct SkillBalanceMetrics {
    started_at: Instant,
    data: Mutex<MetricsData>,
}

impl Default for SkillBalanceMetrics {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            data: Mutex::new(MetricsData::default()),
        }
    }
}

impl SkillBalanceMetrics {
    fn with_data(&self, update: impl FnOnce(&mut MetricsData)) {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        update(&mut data);
    }

    pub(super) fn record_request(&self) {
        self.with_data(|data| data.attack_requests = data.attack_requests.saturating_add(1));
    }

    pub(super) fn record_rejection(&self, reason: AttackRejectReason) {
        self.with_data(|data| data.rejections.record(reason));
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_resolved(
        &self,
        monster_type: &str,
        character_level: u32,
        target_level: u32,
        client_kind: ClientKind,
        weapon_skill: Option<SkillId>,
        skill_level: u32,
        cadence: Option<Duration>,
        hit: bool,
        kill: bool,
        xp: u64,
    ) {
        self.with_data(|data| {
            data.resolved_attacks = data.resolved_attacks.saturating_add(1);
            let Some(weapon_skill) = weapon_skill.filter(|skill| {
                matches!(
                    skill,
                    SkillId::OneHandedSword | SkillId::Dagger | SkillId::Spear
                )
            }) else {
                return;
            };

            data.weapon.record(hit, kill, xp);
            data.weapon_by_skill
                .entry(weapon_skill.as_str().to_string())
                .or_default()
                .record(hit, kill, xp);
            data.weapon_by_skill_band[skill_band(skill_level)].record(hit, kill, xp);
            data.weapon_by_difficulty[difficulty_band(character_level, target_level)]
                .record(hit, kill, xp);
            data.weapon_by_client[client_band(client_kind)].record(hit, kill, xp);
            data.weapon_by_level_pair
                .entry((character_level, skill_band(skill_level)))
                .or_default()
                .record(hit, kill, xp);
            data.weapon_by_monster
                .entry(monster_type.to_string())
                .or_default()
                .record(hit, kill, xp);
            if let Some(cadence) = cadence {
                let milliseconds = cadence.as_millis().min(u128::from(u64::MAX)) as u64;
                data.cadence_samples = data.cadence_samples.saturating_add(1);
                data.cadence_total_ms = data.cadence_total_ms.saturating_add(milliseconds);
                data.cadence_min_ms = Some(
                    data.cadence_min_ms
                        .map_or(milliseconds, |current| current.min(milliseconds)),
                );
                data.cadence_max_ms = Some(
                    data.cadence_max_ms
                        .map_or(milliseconds, |current| current.max(milliseconds)),
                );
            }
        });
    }

    pub(super) fn record_xp_message(&self, skill: SkillId, created_row: bool) {
        self.with_data(|data| {
            if matches!(
                skill,
                SkillId::OneHandedSword | SkillId::Dagger | SkillId::Spear
            ) {
                data.weapon_xp_messages = data.weapon_xp_messages.saturating_add(1);
                data.weapon_rows_created = data
                    .weapon_rows_created
                    .saturating_add(u64::from(created_row));
            } else if skill == SkillId::Shield || armor_skill_construction(skill).is_some() {
                data.defense_xp_messages = data.defense_xp_messages.saturating_add(1);
                data.defense_rows_created = data
                    .defense_rows_created
                    .saturating_add(u64::from(created_row));
            } else if skill == SkillId::Healing {
                data.healing_xp_messages = data.healing_xp_messages.saturating_add(1);
                data.healing_rows_created = data
                    .healing_rows_created
                    .saturating_add(u64::from(created_row));
            }
        });
    }

    pub(super) fn record_defense(
        &self,
        skill: Option<SkillId>,
        skill_level: u32,
        monster_hit: bool,
        xp: u64,
    ) {
        let Some(skill) = skill else {
            return;
        };
        if skill != SkillId::Shield && armor_skill_construction(skill).is_none() {
            return;
        }
        self.with_data(|data| {
            data.defense.record(monster_hit, xp);
            data.defense_by_skill
                .entry(skill.as_str().to_string())
                .or_default()
                .record(monster_hit, xp);
            data.defense_by_skill_band[skill_band(skill_level)].record(monster_hit, xp);
        });
    }

    pub(super) fn record_mitigation(
        &self,
        construction: Option<ArmorConstruction>,
        result: PhysicalDamageResult,
    ) {
        if result.raw_damage == 0 {
            return;
        }
        self.with_data(|data| {
            data.mitigation.record(result);
            data.mitigation_by_type
                .entry(result.damage_type.as_str().to_string())
                .or_default()
                .record(result);
            data.mitigation_by_construction
                .entry(
                    construction
                        .map(|value| value.as_str())
                        .unwrap_or("none")
                        .to_string(),
                )
                .or_default()
                .record(result);
        });
    }

    pub(super) fn record_healing(
        &self,
        skill: Option<SkillId>,
        skill_level: u32,
        restored_hp: u32,
        xp: u64,
    ) {
        let Some(SkillId::Healing) = skill else {
            return;
        };
        self.with_data(|data| {
            data.healing.record(restored_hp, xp);
            data.healing_by_skill_band[skill_band(skill_level)].record(restored_hp, xp);
        });
    }

    pub(super) fn record_save(&self, kind: SkillSaveKind, rows: usize) {
        if rows == 0 {
            return;
        }
        self.with_data(|data| {
            match kind {
                SkillSaveKind::Periodic => {
                    data.saves.periodic_batches = data.saves.periodic_batches.saturating_add(1)
                }
                SkillSaveKind::Logout => {
                    data.saves.logout_batches = data.saves.logout_batches.saturating_add(1)
                }
                SkillSaveKind::Shutdown => {
                    data.saves.shutdown_batches = data.saves.shutdown_batches.saturating_add(1)
                }
            }
            data.saves.rows_written = data.saves.rows_written.saturating_add(rows as u64);
        });
    }

    pub(super) fn snapshot(&self) -> SkillBalanceSnapshot {
        let data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        SkillBalanceSnapshot {
            uptime_secs: self.started_at.elapsed().as_secs(),
            attack_requests: data.attack_requests,
            resolved_attacks: data.resolved_attacks,
            rejections: data.rejections,
            weapon: data.weapon,
            weapon_by_skill: data.weapon_by_skill,
            weapon_by_skill_band: data.weapon_by_skill_band,
            weapon_by_difficulty: data.weapon_by_difficulty,
            weapon_by_client: data.weapon_by_client,
            weapon_by_level_pair: data.weapon_by_level_pair,
            weapon_by_monster: data.weapon_by_monster,
            cadence_samples: data.cadence_samples,
            cadence_total_ms: data.cadence_total_ms,
            cadence_min_ms: data.cadence_min_ms,
            cadence_max_ms: data.cadence_max_ms,
            weapon_xp_messages: data.weapon_xp_messages,
            weapon_rows_created: data.weapon_rows_created,
            defense: data.defense,
            defense_by_skill: data.defense_by_skill,
            defense_by_skill_band: data.defense_by_skill_band,
            defense_xp_messages: data.defense_xp_messages,
            defense_rows_created: data.defense_rows_created,
            mitigation: data.mitigation,
            mitigation_by_type: data.mitigation_by_type,
            mitigation_by_construction: data.mitigation_by_construction,
            healing: data.healing,
            healing_by_skill_band: data.healing_by_skill_band,
            healing_xp_messages: data.healing_xp_messages,
            healing_rows_created: data.healing_rows_created,
            saves: data.saves,
        }
    }
}

impl super::GameState {
    pub fn skill_balance_report(&self) -> String {
        self.skill_balance_metrics
            .snapshot()
            .render(super::combat::PLAYER_ATTACK_COOLDOWN)
    }

    #[cfg(test)]
    pub(super) fn skill_balance_snapshot(&self) -> SkillBalanceSnapshot {
        self.skill_balance_metrics.snapshot()
    }
}

fn skill_band(level: u32) -> usize {
    match level {
        0..=4 => 0,
        5..=14 => 1,
        15..=24 => 2,
        _ => 3,
    }
}

fn difficulty_band(character_level: u32, target_level: u32) -> usize {
    if character_level.saturating_sub(target_level) >= 5 {
        0
    } else if target_level.saturating_sub(character_level) >= 5 {
        2
    } else {
        1
    }
}

fn client_band(kind: ClientKind) -> usize {
    match kind {
        ClientKind::Web => 0,
        ClientKind::Cli => 1,
        ClientKind::Unknown | ClientKind::Other => 2,
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{:.1}m", seconds as f64 / 60.0)
    } else {
        format!("{:.1}h", seconds as f64 / 3_600.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_two_bands_cover_skill_difficulty_and_client_boundaries() {
        assert_eq!([skill_band(0), skill_band(4), skill_band(5)], [0, 0, 1]);
        assert_eq!([skill_band(14), skill_band(15), skill_band(24)], [1, 2, 2]);
        assert_eq!([skill_band(25), skill_band(30), skill_band(999)], [3, 3, 3]);
        assert_eq!(difficulty_band(10, 5), 0);
        assert_eq!(difficulty_band(10, 6), 1);
        assert_eq!(difficulty_band(6, 10), 1);
        assert_eq!(difficulty_band(5, 10), 2);
        assert_eq!(client_band(ClientKind::Web), 0);
        assert_eq!(client_band(ClientKind::Cli), 1);
        assert_eq!(client_band(ClientKind::Other), 2);
    }

    #[test]
    fn report_projects_progress_from_observed_xp_and_cadence() {
        let metrics = SkillBalanceMetrics::default();
        metrics.record_request();
        metrics.record_resolved(
            "kobold",
            1,
            1,
            ClientKind::Cli,
            Some(SkillId::OneHandedSword),
            5,
            Some(Duration::from_millis(1_600)),
            true,
            false,
            10,
        );
        metrics.record_xp_message(SkillId::OneHandedSword, true);
        metrics.record_save(SkillSaveKind::Periodic, 1);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.attack_requests, 1);
        assert_eq!(
            snapshot.weapon,
            AttackStats {
                attacks: 1,
                hits: 1,
                kills: 0,
                xp: 10
            }
        );
        assert_eq!(snapshot.weapon_by_skill["one_handed_sword"].attacks, 1);
        assert_eq!(snapshot.weapon_by_skill_band[1].attacks, 1);
        assert_eq!(snapshot.weapon_by_difficulty[1].xp, 10);
        assert_eq!(snapshot.weapon_by_client[1].attacks, 1);
        assert_eq!(snapshot.weapon_by_level_pair[&(1, 1)].attacks, 1);
        assert_eq!(snapshot.weapon_by_monster["kobold"].xp, 10);
        assert_eq!(snapshot.weapon_xp_messages, 1);
        assert_eq!(snapshot.weapon_rows_created, 1);
        assert_eq!(snapshot.saves.periodic_batches, 1);
        assert_eq!(snapshot.saves.rows_written, 1);

        let report = snapshot.render(Duration::from_millis(1_533));
        assert!(report.contains("hit_rate=100.0%"));
        assert!(report.contains("lv5=550attacks/14.7m"));
        assert!(report.contains("kobold:10xp/1a"));
    }
}

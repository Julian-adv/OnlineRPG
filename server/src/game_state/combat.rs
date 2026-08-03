use crate::game::{character_hp, combat};
use crate::types::{
    AttackRejectReason, ClientKind, MonsterState, PlayerId, Position, ServerMessage,
};
use onlinerpg_shared::combat::resolve_physical_damage;
use onlinerpg_shared::inventory::{ArmorConstruction, EquipSlot, GroundItem, PlayerInventory};
use onlinerpg_shared::skills::{
    armor_skill_guard_bonus, shield_skill_guard_bonus, weapon_skill_attack_bonus,
    weapon_skill_attack_cooldown_ms, weapon_skill_melee_range, SkillId,
    DEFAULT_WEAPON_ATTACK_COOLDOWN_MS, DEFAULT_WEAPON_MELEE_RANGE_METERS,
};
use onlinerpg_shared::xp;
use onlinerpg_shared::PhysicalDamageType;
use rand::Rng;
use std::f32::consts::TAU;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const WEAPON_DROP_OFFSET_METERS: f32 = 2.0;
// Mirrors the web and agent clients' 2m melee reach. This server-side check is
// authoritative: clients may request an attack directly without chasing.
// Default for unarmed/unmapped attacks and the Phase 2 report's empty-sample
// projection. Mapped weapons carry their own shared range and clip cadence.
pub(super) const PLAYER_ATTACK_COOLDOWN: Duration =
    Duration::from_millis(DEFAULT_WEAPON_ATTACK_COOLDOWN_MS as u64);
// Out-of-range swings may still pull aggro when the monster is plausibly
// nearby, but farther requests are ignored to prevent remote provocation.
pub(super) const PLAYER_ATTACK_PROVOKE_RANGE_METERS: f32 = 10.0;
// Slack added to a monster's own attack_range when validating an owner-reported
// hit. Monster movement is simulated by the owning client, so its position and
// the target's can both lag the server's view by a round-trip; this absorbs that
// drift without leaving the reach unbounded.
const MONSTER_ATTACK_RANGE_TOLERANCE_METERS: f32 = 4.0;
// Authored timing data the clients also import (data-src/player_anim_timing.csv),
// so server delays can never drift from the animations.
static PLAYER_ANIM_TIMING: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../data/player_anim_timing.json"))
        .expect("player_anim_timing.json parses")
});

fn anim_delay_ms(id: &str) -> u64 {
    PLAYER_ANIM_TIMING[id]["delayMs"]
        .as_u64()
        .unwrap_or_else(|| panic!("{id}.delayMs is a number"))
}

// How long into the swing the blade lands.
pub(super) static PLAYER_ATTACK_IMPACT_DELAY: LazyLock<Duration> =
    LazyLock::new(|| Duration::from_millis(anim_delay_ms("player_attack_impact")));

// Slash1 cadence (clip lasts 1,533ms) minus a server-arrival jitter allowance.
pub(super) static PLAYER_ATTACK_INTERVAL_MS: LazyLock<u64> =
    LazyLock::new(|| anim_delay_ms("player_attack_interval"));

fn dropped_weapon_position(monster_position: Position) -> Position {
    let angle = rand::thread_rng().gen_range(0.0..TAU);
    offset_position_at_angle(monster_position, angle, WEAPON_DROP_OFFSET_METERS)
}

/// Squared XZ distance between two participants, or `None` when they sit on
/// different floors or a position is non-finite. Attack and trade paths gate on
/// this before applying their own reach: floors are stacked (a dungeon depth
/// runs directly under the overworld), so a same-XZ neighbour one floor away
/// must never read as reachable.
pub(super) fn reachable_dist_sq(a: Position, a_floor: i8, b: Position, b_floor: i8) -> Option<f32> {
    if a_floor != b_floor {
        return None;
    }
    let dist_sq = a.dist_xz_sq(&b);
    dist_sq.is_finite().then_some(dist_sq)
}

pub(super) fn offset_position_at_angle(origin: Position, angle: f32, distance: f32) -> Position {
    Position {
        x: origin.x + angle.cos() * distance,
        y: origin.y,
        z: origin.z + angle.sin() * distance,
    }
}

/// Everything the attack roll needs once a `PlayerAttack` request has cleared
/// every gate in `validate_player_attack`.
struct PlayerAttackContext {
    monster_type: String,
    monster_position: Position,
    monster_floor_level: i8,
    monster_level_override: Option<u8>,
    player_name: String,
    player_level: u32,
    client_kind: ClientKind,
    accepted_cadence: Option<Duration>,
    weapon: PlayerWeaponAttackProfile,
}

pub(super) struct PlayerWeaponAttackProfile {
    pub(super) damage_dice: String,
    pub(super) damage_type: PhysicalDamageType,
    pub(super) enchant: i32,
    pub(super) weapon_skill: Option<SkillId>,
    pub(super) weapon_skill_level: u32,
    pub(super) weapon_skill_attack_bonus: i32,
    pub(super) melee_range: f32,
    pub(super) attack_cooldown: Duration,
}

pub(super) struct PlayerDefenseProfile {
    pub(super) effective_guard: i32,
    pub(super) primary_armor_construction: Option<ArmorConstruction>,
    pub(super) shield_skill: Option<SkillId>,
    pub(super) shield_skill_level: u32,
    pub(super) shield_skill_guard_bonus: i32,
    pub(super) armor_skill: Option<SkillId>,
    pub(super) armor_skill_level: u32,
    pub(super) armor_skill_guard_bonus: i32,
}

impl super::GameState {
    pub(super) async fn player_weapon_attack_profile(
        &self,
        player_id: &PlayerId,
    ) -> PlayerWeaponAttackProfile {
        let equipped_weapon = {
            let inventories = self.inventories.read().await;
            inventories
                .get(player_id)
                .and_then(|inv| inv.equipped.get(&EquipSlot::MainHand))
                .map(|item| (item.item_def_id.clone(), item.enchant))
        };
        let Some((item_def_id, enchant)) = equipped_weapon else {
            return PlayerWeaponAttackProfile {
                damage_dice: "1d2".to_string(),
                damage_type: PhysicalDamageType::Untyped,
                enchant: 0,
                weapon_skill: None,
                weapon_skill_level: 0,
                weapon_skill_attack_bonus: 0,
                melee_range: DEFAULT_WEAPON_MELEE_RANGE_METERS,
                attack_cooldown: PLAYER_ATTACK_COOLDOWN,
            };
        };
        let Some(def) = self.item_defs.get(&item_def_id) else {
            return PlayerWeaponAttackProfile {
                damage_dice: "1d2".to_string(),
                damage_type: PhysicalDamageType::Untyped,
                enchant: 0,
                weapon_skill: None,
                weapon_skill_level: 0,
                weapon_skill_attack_bonus: 0,
                melee_range: DEFAULT_WEAPON_MELEE_RANGE_METERS,
                attack_cooldown: PLAYER_ATTACK_COOLDOWN,
            };
        };
        let Some(damage_dice) = def.damage_dice() else {
            return PlayerWeaponAttackProfile {
                damage_dice: "1d2".to_string(),
                damage_type: PhysicalDamageType::Untyped,
                enchant: 0,
                weapon_skill: None,
                weapon_skill_level: 0,
                weapon_skill_attack_bonus: 0,
                melee_range: DEFAULT_WEAPON_MELEE_RANGE_METERS,
                attack_cooldown: PLAYER_ATTACK_COOLDOWN,
            };
        };
        let weapon_skill = def.weapon_skill;
        let weapon_skill_level = if let Some(skill) = weapon_skill {
            self.skill_level(player_id, skill).await
        } else {
            0
        };
        let weapon_skill_attack_bonus = weapon_skill
            .map(|skill| weapon_skill_attack_bonus(skill, weapon_skill_level))
            .unwrap_or_default();
        let melee_range = weapon_skill
            .map(weapon_skill_melee_range)
            .unwrap_or(DEFAULT_WEAPON_MELEE_RANGE_METERS);
        let attack_cooldown = weapon_skill
            .map(weapon_skill_attack_cooldown_ms)
            .map(|milliseconds| Duration::from_millis(u64::from(milliseconds)))
            .unwrap_or(PLAYER_ATTACK_COOLDOWN);
        PlayerWeaponAttackProfile {
            damage_dice: damage_dice.to_string(),
            damage_type: def.damage_type.unwrap_or(PhysicalDamageType::Untyped),
            enchant,
            weapon_skill,
            weapon_skill_level,
            weapon_skill_attack_bonus,
            melee_range,
            attack_cooldown,
        }
    }

    /// Put a kill's loot — the weapon drop and any rare world drops — on the
    /// ground once the killing blow lands. The wait lives server-side: a
    /// ground item is pickable the moment it exists, so a client that skips
    /// the animation must not reach it early.
    pub(super) fn spawn_kill_loot_after_impact(
        &self,
        weapon_drop: Option<GroundItem>,
        origin: Position,
        floor_level: i8,
    ) {
        let game_state = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(*PLAYER_ATTACK_IMPACT_DELAY).await;
            if let Some(item) = weapon_drop {
                game_state.spawn_ground_item(item).await;
            }
            game_state.spawn_world_drops(origin, floor_level).await;
        });
    }

    async fn claim_player_attack_window(&self, player_id: &PlayerId) -> bool {
        let now = Self::now_ms();
        let mut last_attacks = self.last_player_attacks.write().await;
        let last = last_attacks.entry(*player_id).or_insert(0);
        if now.saturating_sub(*last) < *PLAYER_ATTACK_INTERVAL_MS {
            return false;
        }
        *last = now;
        true
    }

    /// Sum of the guard bonuses from every equipped item — the single place
    /// that maps equipped gear to a guard number. Pure over the loaded item
    /// definitions; `effective_guard` adds it to the base attribute.
    fn equipped_guard_bonus(&self, inv: &PlayerInventory) -> i32 {
        inv.equipped
            .values()
            .filter_map(|item| self.item_defs.get(&item.item_def_id))
            .filter_map(|def| def.guard)
            .sum()
    }

    pub(super) async fn player_defense_profile(
        &self,
        player_id: &PlayerId,
    ) -> PlayerDefenseProfile {
        let base_guard = {
            let chars = self.player_characters.read().await;
            chars
                .get(player_id)
                .map(|(_, _, attrs)| i32::from(attrs.guard))
                .unwrap_or(10)
        };
        let (equipment_guard, shield_skill, armor_skill, primary_armor_construction) = {
            let inventories = self.inventories.read().await;
            inventories
                .get(player_id)
                .map_or((0, None, None, None), |inventory| {
                    let shield_skill = inventory
                        .equipped
                        .get(&EquipSlot::OffHand)
                        .and_then(|item| self.item_defs.get(&item.item_def_id))
                        .and_then(|def| def.defense_skill);
                    let armor_skill = inventory
                        .equipped
                        .get(&EquipSlot::Chest)
                        .and_then(|item| self.item_defs.get(&item.item_def_id))
                        .and_then(|def| def.defense_skill);
                    let primary_armor_construction = inventory
                        .equipped
                        .get(&EquipSlot::Chest)
                        .and_then(|item| self.item_defs.get(&item.item_def_id))
                        .and_then(|def| def.armor_construction);
                    (
                        self.equipped_guard_bonus(inventory),
                        shield_skill,
                        armor_skill,
                        primary_armor_construction,
                    )
                })
        };
        let shield_skill_level = if let Some(skill) = shield_skill {
            self.skill_level(player_id, skill).await
        } else {
            0
        };
        let shield_skill_guard_bonus = match shield_skill {
            Some(SkillId::Shield) => shield_skill_guard_bonus(shield_skill_level),
            _ => 0,
        };
        let armor_skill_level = if let Some(skill) = armor_skill {
            self.skill_level(player_id, skill).await
        } else {
            0
        };
        let armor_skill_guard_bonus = armor_skill
            .map(|skill| armor_skill_guard_bonus(skill, armor_skill_level))
            .unwrap_or_default();
        PlayerDefenseProfile {
            effective_guard: base_guard
                + equipment_guard
                + shield_skill_guard_bonus
                + armor_skill_guard_bonus,
            primary_armor_construction,
            shield_skill,
            shield_skill_level,
            shield_skill_guard_bonus,
            armor_skill,
            armor_skill_level,
            armor_skill_guard_bonus,
        }
    }

    /// A player's effective guard: base attribute, equipped-gear bonuses, and
    /// one explicitly mapped defensive-skill modifier.
    /// This is exactly the target number an attacker must beat to land a hit,
    /// and the value reported to the client so it never has to recompute the
    /// formula itself.
    pub async fn effective_guard(&self, player_id: &PlayerId) -> i32 {
        self.player_defense_profile(player_id).await.effective_guard
    }

    /// Runs every gate on a `PlayerAttack` request. `Err` is the coarse reason
    /// acked back to the attacker at the single call site, so a new gate can
    /// never silently drop a request. Side effect: an out-of-range swing
    /// within provoke range still aggros the monster onto the attacker.
    async fn validate_player_attack(
        &self,
        player_id: &PlayerId,
        monster_id: &str,
    ) -> Result<PlayerAttackContext, AttackRejectReason> {
        let monster_snapshot = {
            let monsters = self.monsters.read().await;
            monsters
                .get(monster_id)
                .filter(|m| m.state != MonsterState::Dead)
                .map(|m| {
                    (
                        m.monster_type.clone(),
                        m.position,
                        m.floor_level,
                        m.level_override,
                        m.owner_id,
                    )
                })
        };
        let Some((
            monster_type,
            monster_position,
            monster_floor_level,
            monster_level_override,
            monster_owner_id,
        )) = monster_snapshot
        else {
            return Err(AttackRejectReason::InvalidTarget);
        };

        let player_snapshot = {
            let players = self.players.read().await;
            players.get(player_id).map(|p| {
                (
                    p.name.clone(),
                    p.level,
                    p.floor_level,
                    p.position,
                    p.health,
                    p.client_kind,
                )
            })
        };
        let Some((
            player_name,
            player_level,
            player_floor,
            player_position,
            player_health,
            client_kind,
        )) = player_snapshot
        else {
            warn!("Attack from non-existent player: {}", player_id);
            return Err(AttackRejectReason::NotInGame);
        };

        // A dead attacker deals no damage. The monster-attack path already
        // gates on the target's HP; mirror it here so a 0-HP player can't
        // keep swinging while awaiting respawn.
        if player_health == 0 {
            return Err(AttackRejectReason::AttackerDead);
        }
        let weapon = self.player_weapon_attack_profile(player_id).await;
        // Delivery filtering keeps a player from ever learning about
        // monsters on another floor, but gate here too so a stale monster
        // id can't drive a cross-floor hit (the original bug: a surface
        // guard striking a monster on the dungeon floor beneath it).
        let Some(distance_sq) = reachable_dist_sq(
            player_position,
            player_floor,
            monster_position,
            monster_floor_level,
        ) else {
            return Err(AttackRejectReason::InvalidTarget);
        };
        if distance_sq > weapon.melee_range.powi(2) {
            if distance_sq <= PLAYER_ATTACK_PROVOKE_RANGE_METERS.powi(2) {
                if let Some(owner_id) = monster_owner_id {
                    self.send_direct_message(
                        &owner_id,
                        ServerMessage::MonsterProvoked {
                            player_id: *player_id,
                            monster_id: monster_id.to_string(),
                        },
                    )
                    .await;
                }
            }
            return Err(AttackRejectReason::OutOfRange);
        }

        let now = Instant::now();
        let mut attack_times = self.player_attack_times.write().await;
        if attack_times
            .get(player_id)
            .is_some_and(|last| now.saturating_duration_since(*last) < weapon.attack_cooldown)
        {
            return Err(AttackRejectReason::Cooldown);
        }
        let accepted_cadence = attack_times
            .insert(*player_id, now)
            .map(|previous| now.saturating_duration_since(previous));

        Ok(PlayerAttackContext {
            monster_type,
            monster_position,
            monster_floor_level,
            monster_level_override,
            player_name,
            player_level,
            client_kind,
            accepted_cadence,
            weapon,
        })
    }

    async fn reject_player_attack(
        &self,
        player_id: &PlayerId,
        monster_id: String,
        reason: AttackRejectReason,
    ) {
        self.skill_balance_metrics.record_rejection(reason);
        self.send_direct_message(
            player_id,
            ServerMessage::PlayerAttackRejected { monster_id, reason },
        )
        .await;
    }

    pub async fn broadcast_player_attack(&self, player_id: &PlayerId, monster_id: String) {
        self.skill_balance_metrics.record_request();
        let PlayerAttackContext {
            monster_type,
            monster_position,
            monster_floor_level,
            monster_level_override,
            player_name,
            player_level,
            client_kind,
            accepted_cadence,
            weapon,
        } = match self.validate_player_attack(player_id, &monster_id).await {
            Ok(ctx) => ctx,
            Err(reason) => {
                self.reject_player_attack(player_id, monster_id, reason)
                    .await;
                return;
            }
        };
        if !self.claim_player_attack_window(player_id).await {
            return;
        }
        // A landed attack (not a rejected one) breaks concentration.
        self.cancel_concentration_if_active(player_id).await;
        debug!("Player {} attacking monster {}", player_name, monster_id);

        debug!(
            "Weapon profile: skill={:?}, level={}, attack bonus={}",
            weapon.weapon_skill, weapon.weapon_skill_level, weapon.weapon_skill_attack_bonus
        );
        let strength = {
            let chars = self.player_characters.read().await;
            chars
                .get(player_id)
                .map(|(_, _, attrs)| attrs.r#str)
                .unwrap_or(10)
        };

        let (result_hit, result_roll, result_damage) = {
            let def = self.monster_defs.get(&monster_type);
            let target_guard = def.map(|d| i32::from(d.guard)).unwrap_or(10);
            let attack_bonus = combat::player_attack_bonus(
                player_level,
                strength,
                weapon.enchant,
                weapon.weapon_skill_attack_bonus,
            );
            let result = combat::roll_attack(
                attack_bonus,
                target_guard,
                &weapon.damage_dice,
                combat::player_damage_bonus(strength, weapon.enchant),
            );
            (result.hit, result.roll, result.damage)
        };

        debug!(
            "Dice roll: {}, Hit: {}, Damage: {}",
            result_roll, result_hit, result_damage
        );

        let killing_blow = {
            let mut monsters = self.monsters.write().await;
            let Some(monster) = monsters.get_mut(&monster_id) else {
                drop(monsters);
                self.reject_player_attack(player_id, monster_id, AttackRejectReason::InvalidTarget)
                    .await;
                return;
            };
            if monster.state == MonsterState::Dead {
                drop(monsters);
                self.reject_player_attack(player_id, monster_id, AttackRejectReason::InvalidTarget)
                    .await;
                return;
            }
            if result_hit {
                monster.health = monster.health.saturating_sub(result_damage);
                debug!(
                    "Monster {} HP: {}/{}",
                    monster_id, monster.health, monster.max_health
                );
                if monster.health == 0 {
                    monster.state = MonsterState::Dead;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        // Update player combat timestamp and damage logic
        {
            let mut players = self.players.write().await;
            if let Some(player) = players.get_mut(player_id) {
                player.last_combat_at = Self::now_ms();
            }
        }

        // Send attack result
        self.send_direct_message_to_players_within_position(
            &monster_position,
            monster_floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::PlayerAttacked {
                player_id: *player_id,
                monster_id: monster_id.clone(),
                hit: result_hit,
                roll: result_roll,
                damage_type: weapon.damage_type,
                damage: result_damage,
            },
            None,
        )
        .await;

        if killing_blow {
            let dropped_weapon_item_def_id = self
                .monster_defs
                .get(&monster_type)
                .filter(|def| {
                    def.weapon_drop_chance >= 1.0
                        || rand::thread_rng().gen::<f32>() < def.weapon_drop_chance
                })
                .and_then(|def| def.weapon.as_deref())
                .and_then(|weapon| self.item_defs.item_def_id_for_weapon_ref(weapon));

            debug!("Monster {} died, broadcasting dead state", monster_id);
            self.send_direct_message_to_players_within_position(
                &monster_position,
                monster_floor_level,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::MonsterDead {
                    monster_id: monster_id.clone(),
                    dropped_weapon_item_def_id: dropped_weapon_item_def_id.clone(),
                },
                None,
            )
            .await;

            let weapon_drop = if let Some(item_def_id) = dropped_weapon_item_def_id {
                let instance_id = self.next_instance_id().await;
                // Scatter off the corpse, then clamp onto walkable dungeon
                // floor: pickup is a pure proximity check, so an item behind
                // a wall would be lost.
                let drop_position = self
                    .loot_drop_position(
                        monster_position,
                        monster_floor_level,
                        dropped_weapon_position(monster_position),
                    )
                    .await;
                Some(GroundItem {
                    instance_id,
                    item_def_id,
                    position: drop_position,
                    floor_level: monster_floor_level,
                    enchant: 0,
                })
            } else {
                None
            };
            // Weapon drop and rare bonus world drops alike wait for the blow.
            self.spawn_kill_loot_after_impact(weapon_drop, monster_position, monster_floor_level);

            // Dungeon monsters: free their spawn slot for respawn.
            self.on_dungeon_monster_dead(&monster_id).await;

            // Award XP to the player who killed the monster.
            // Depth-scaled dungeon monsters yield XP for their
            // effective level, not the base definition level.
            let xp_def = self.monster_defs.get(&monster_type);
            if let Some(def) = xp_def {
                let effective_level = monster_level_override.unwrap_or(def.level);
                let xp_amount = xp::monster_xp(effective_level, def.guard);
                let player_char = {
                    let map = self.player_characters.read().await;
                    map.get(player_id).cloned()
                };
                if let Some((_, old_xp, attributes)) = player_char {
                    let new_xp = old_xp + xp_amount as u64;
                    let old_level = xp::level_from_xp(old_xp);
                    let new_level = xp::level_from_xp(new_xp);
                    let leveled_up = new_level > old_level;
                    let levels_gained = new_level.saturating_sub(old_level);

                    // Update in-memory XP
                    {
                        let mut map = self.player_characters.write().await;
                        if let Some(entry) = map.get_mut(player_id) {
                            entry.1 = new_xp;
                        }
                    }

                    // Update level/max HP in player map if leveled up
                    let mut new_max_hp = None;
                    let mut new_current_hp = None;
                    if leveled_up {
                        let mut players_write = self.players.write().await;
                        if let Some(p) = players_write.get_mut(player_id) {
                            p.level = new_level;
                            let mut updated_max_hp = p.max_health;
                            for _ in 0..levels_gained {
                                match character_hp::level_up_max_hp(
                                    updated_max_hp,
                                    &p.class,
                                    attributes.con,
                                ) {
                                    Ok(next_max_hp) => {
                                        updated_max_hp = next_max_hp;
                                    }
                                    Err(err) => {
                                        warn!(
                                            "Failed to roll level-up HP for player {}: {}",
                                            player_name, err
                                        );
                                        break;
                                    }
                                }
                            }

                            if updated_max_hp != p.max_health {
                                p.max_health = updated_max_hp;
                                new_max_hp = Some(updated_max_hp);
                            }

                            // Level-up always fully restores current HP to max HP.
                            p.health = p.max_health;
                            new_current_hp = Some(p.health);
                        }
                    }

                    // Mark dirty for periodic batch save
                    self.mark_dirty(player_id).await;

                    // Notify the player directly
                    let max_hp_for_msg = if let Some(max_hp) = new_max_hp {
                        max_hp
                    } else {
                        self.players
                            .read()
                            .await
                            .get(player_id)
                            .map(|p| p.max_health)
                            .unwrap_or(0)
                    };
                    let current_hp_for_msg = if let Some(current_hp) = new_current_hp {
                        current_hp
                    } else {
                        self.players
                            .read()
                            .await
                            .get(player_id)
                            .map(|p| p.health)
                            .unwrap_or(0)
                    };
                    self.send_direct_message(
                        player_id,
                        ServerMessage::XpGained {
                            player_id: *player_id,
                            xp_amount,
                            xp_lost: 0,
                            total_xp: new_xp,
                            new_level,
                            leveled_up,
                            max_hp: max_hp_for_msg,
                            current_hp: current_hp_for_msg,
                        },
                    )
                    .await;

                    debug!(
                        "Player {} gained {} XP (total: {}, level: {}{})",
                        player_name,
                        xp_amount,
                        new_xp,
                        new_level,
                        if leveled_up { " LEVEL UP!" } else { "" }
                    );
                }
            }

            // Schedule removal after 30 seconds
            let game_state = self.clone();
            let id_to_remove = monster_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                let mut monsters = game_state.monsters.write().await;
                if let Some(monster) = monsters.get(&id_to_remove) {
                    if monster.state == MonsterState::Dead {
                        let monster_position = monster.position;
                        let monster_floor = monster.floor_level;
                        monsters.remove(&id_to_remove);
                        drop(monsters);
                        debug!("Monster {} removed after 30s corpse time", id_to_remove);
                        game_state
                            .send_direct_message_to_players_within_position(
                                &monster_position,
                                monster_floor,
                                super::EVENT_DELIVERY_RADIUS,
                                ServerMessage::MonsterRemoved {
                                    monster_id: id_to_remove,
                                },
                                None,
                            )
                            .await;
                    }
                }
            });
        }

        let skill_xp = if let Some(skill) = weapon.weapon_skill {
            let amount = combat::weapon_skill_attack_xp(result_hit, killing_blow);
            self.add_skill_xp(player_id, skill, amount)
                .await
                .map_or(0, |result| result.xp_amount)
        } else {
            0
        };
        let target_level = self.monster_defs.get(&monster_type).map_or(1, |def| {
            u32::from(monster_level_override.unwrap_or(def.level))
        });
        self.skill_balance_metrics.record_resolved(
            &monster_type,
            player_level,
            target_level,
            client_kind,
            weapon.weapon_skill,
            weapon.weapon_skill_level,
            accepted_cadence,
            result_hit,
            killing_blow,
            skill_xp,
        );
    }

    pub async fn broadcast_monster_attack(
        &self,
        attacker_player_id: &PlayerId,
        monster_id: &str,
        target_player_id: &PlayerId,
    ) {
        // 1. Check if monster exists, is alive, and is owned by the requester.
        // Also check server-side cooldown guard.
        let now = Self::now_ms();
        let mut monster_data = None;

        {
            let mut monsters = self.monsters.write().await;
            if let Some(monster) = monsters.get_mut(monster_id) {
                if monster.is_controllable_by(attacker_player_id) {
                    let def = self.monster_defs.get(&monster.monster_type);
                    let attack_cooldown_ms =
                        def.map(|d| u64::from(d.attack_cooldown)).unwrap_or(1500);

                    if now.saturating_sub(monster.last_attack_at) >= attack_cooldown_ms {
                        monster.last_attack_at = now;
                        let weapon_damage_roll = def
                            .and_then(|d| d.weapon.as_deref())
                            .and_then(|weapon| self.item_defs.damage_dice_for_weapon_model(weapon));
                        let damage_type = def
                            .map(|d| {
                                d.weapon
                                    .as_deref()
                                    .and_then(|weapon| {
                                        self.item_defs.damage_type_for_weapon_ref(weapon)
                                    })
                                    .unwrap_or_else(|| d.damage_type())
                            })
                            .unwrap_or(PhysicalDamageType::Untyped);
                        // Depth-scaled dungeon monsters attack at their
                        // effective level (bonus + damage dice).
                        let (attack_bonus, damage_roll) = match monster.level_override {
                            Some(level) => (
                                combat::level_attack_bonus(u32::from(level)),
                                combat::monster_damage_roll_for_level(level).to_string(),
                            ),
                            None => (
                                def.map(|d| d.attack_bonus())
                                    .unwrap_or_else(|| combat::level_attack_bonus(1)),
                                def.map(|d| d.damage_roll())
                                    .unwrap_or_else(|| "1d6".to_string()),
                            ),
                        };
                        monster_data = Some((
                            attack_bonus,
                            damage_roll,
                            weapon_damage_roll,
                            damage_type,
                            monster.position,
                            monster.floor_level,
                            def.map(|d| d.attack_range)
                                .unwrap_or(onlinerpg_shared::monster_ai::DEFAULT_ATTACK_RANGE),
                        ));
                    }
                }
            }
        }

        let (
            attack_bonus,
            damage_roll,
            weapon_damage_roll,
            damage_type,
            monster_position,
            monster_floor_level,
            monster_attack_range,
        ) = match monster_data {
            Some(data) => data,
            None => return,
        };

        // 2. Check if target player exists and is alive
        let (target_player_name, target_position, target_floor_level);
        {
            let players = self.players.read().await;
            match players.get(target_player_id) {
                Some(player) if player.health > 0 => {
                    target_player_name = player.name.clone();
                    target_position = player.position;
                    target_floor_level = player.floor_level;
                }
                _ => return,
            }
        }

        // 3. The monster must actually be able to reach the target. Ownership
        // alone is not enough: any player can spawn a monster next to themselves
        // and become its owner, so without this the pair (monster_id, arbitrary
        // target_player_id) would deal real damage at unlimited range to anyone
        // whose id the attacker can name.
        let Some(distance_sq) = reachable_dist_sq(
            monster_position,
            monster_floor_level,
            target_position,
            target_floor_level,
        ) else {
            return;
        };
        let max_range = monster_attack_range + MONSTER_ATTACK_RANGE_TOLERANCE_METERS;
        if distance_sq > max_range.powi(2) {
            debug!(
                "Rejected monster attack {:.0}m away: monster {} -> player {}",
                distance_sq.sqrt(),
                monster_id,
                target_player_name
            );
            return;
        }
        let defense = self.player_defense_profile(target_player_id).await;

        let result = combat::roll_attack_with_extra_damage_roll(
            attack_bonus,
            defense.effective_guard,
            &damage_roll,
            weapon_damage_roll.as_deref(),
            0,
        );
        let damage = resolve_physical_damage(
            result.damage,
            damage_type,
            defense.primary_armor_construction,
        );
        self.skill_balance_metrics
            .record_mitigation(defense.primary_armor_construction, damage);

        debug!(
            "Monster {} attacks player {}: Roll {}, Hit: {}, Damage: {} {} -> {} (mitigated {})",
            monster_id,
            target_player_name,
            result.roll,
            result.hit,
            damage.raw_damage,
            damage.damage_type.as_str(),
            damage.final_damage,
            damage.mitigated_damage
        );

        // Update player HP and combat timestamp
        let mut did_die = false;
        let mut current_health = 0;
        let mut target_loc: Option<(Position, i8)> = None;

        {
            let mut players = self.players.write().await;
            if let Some(player) = players.get_mut(target_player_id) {
                if player.health == 0 {
                    return; // Already dead
                }

                player.last_combat_at = now;

                if result.hit {
                    player.health = player.health.saturating_sub(damage.final_damage);
                    if player.health == 0 {
                        did_die = true;
                    }
                }
                current_health = player.health;
                target_loc = Some((player.position, player.floor_level));
            }
        }

        if result.hit {
            self.mark_dirty(target_player_id).await;
        }

        let mut guard_bonus_changed = false;
        let shield_xp = if defense.shield_skill == Some(SkillId::Shield) {
            let amount = combat::shield_skill_defense_xp(result.hit);
            let xp_result = self
                .add_skill_xp(target_player_id, SkillId::Shield, amount)
                .await;
            if xp_result.is_some_and(|xp| {
                shield_skill_guard_bonus(xp.new_level) != defense.shield_skill_guard_bonus
            }) {
                guard_bonus_changed = true;
            }
            xp_result.map_or(0, |xp| xp.xp_amount)
        } else {
            0
        };
        self.skill_balance_metrics.record_defense(
            defense.shield_skill,
            defense.shield_skill_level,
            result.hit,
            shield_xp,
        );

        let armor_xp = if defense.armor_skill == Some(SkillId::LeatherArmor) {
            let amount = combat::armor_skill_defense_xp(result.hit);
            let xp_result = if amount > 0 {
                self.add_skill_xp(target_player_id, SkillId::LeatherArmor, amount)
                    .await
            } else {
                None
            };
            if xp_result.is_some_and(|xp| {
                armor_skill_guard_bonus(SkillId::LeatherArmor, xp.new_level)
                    != defense.armor_skill_guard_bonus
            }) {
                guard_bonus_changed = true;
            }
            xp_result.map_or(0, |xp| xp.xp_amount)
        } else {
            0
        };
        self.skill_balance_metrics.record_defense(
            defense.armor_skill,
            defense.armor_skill_level,
            result.hit,
            armor_xp,
        );

        if guard_bonus_changed {
            // Send once after every XP delta so simultaneous Shield/armor
            // thresholds publish the final combined Guard.
            self.send_direct_message(
                target_player_id,
                ServerMessage::GuardUpdated {
                    guard: self.effective_guard(target_player_id).await,
                },
            )
            .await;
        }

        // Send attack result after server-side HP update.
        let attack_msg = ServerMessage::MonsterAttackedPlayer {
            monster_id: monster_id.to_string(),
            player_id: *target_player_id,
            hit: result.hit,
            roll: result.roll,
            damage_type: damage.damage_type,
            raw_damage: damage.raw_damage,
            mitigated_damage: damage.mitigated_damage,
            damage: damage.final_damage,
            current_health,
        };
        if let Some((target_position, target_floor)) = target_loc {
            self.send_direct_message_to_players_within_position(
                &target_position,
                target_floor,
                super::EVENT_DELIVERY_RADIUS,
                attack_msg,
                None,
            )
            .await;
        } else {
            self.send_direct_message(target_player_id, attack_msg).await;
        }

        if did_die {
            let dead_player_id = *target_player_id;
            self.on_player_died(&dead_player_id).await;
            if let Some((target_position, target_floor)) = target_loc {
                self.send_direct_message_to_players_within_position(
                    &target_position,
                    target_floor,
                    super::EVENT_DELIVERY_RADIUS,
                    ServerMessage::PlayerDead {
                        player_id: dead_player_id,
                    },
                    None,
                )
                .await;
            }
        }
    }

    pub async fn tick_regeneration(&self) {
        let mut updates = Vec::new();

        // Weak-from-hunger or food-poisoned players don't regenerate
        // (doc/HUNGER.md); potions remain the escape hatch.
        let regen_blocked = self.hunger_regen_blocked().await;
        {
            let players = self.players.read().await;
            let player_chars = self.player_characters.read().await;
            let now = Self::now_ms();

            for (player_id, player) in players.iter() {
                // Only regenerate if alive and wounded
                if player.health > 0 && player.health < player.max_health {
                    if now.saturating_sub(player.last_combat_at) < super::OUT_OF_COMBAT_MS {
                        continue;
                    }
                    if regen_blocked.contains(player_id) {
                        continue;
                    }

                    let con = player_chars
                        .get(player_id)
                        .map(|(_, _, attrs)| attrs.con)
                        .unwrap_or(10); // Default to 10 if not found

                    let con_mod = (i16::from(con) - 10) / 2;
                    let amount = (1 + (player.level as i32 / 5) + con_mod as i32).max(1) as u32;

                    updates.push((*player_id, amount));
                }
            }
        }

        if updates.is_empty() {
            return;
        }

        let mut regen_dirty: Vec<PlayerId> = Vec::new();
        let mut regen_messages = Vec::new();
        {
            let mut players = self.players.write().await;
            for (player_id, amount) in updates {
                if let Some(player) = players.get_mut(&player_id) {
                    if player.health > 0 && player.health < player.max_health {
                        let old_health = player.health;
                        player.health = (player.health + amount).min(player.max_health);

                        if player.health != old_health {
                            regen_dirty.push(player_id);
                            let position = player.position;
                            let floor_level = player.floor_level;
                            regen_messages.push((
                                position,
                                floor_level,
                                ServerMessage::PlayerHealthUpdate {
                                    player_id,
                                    health: player.health,
                                    max_health: player.max_health,
                                },
                            ));
                        }
                    }
                }
            }
        }
        for (position, floor_level, msg) in regen_messages {
            self.send_direct_message_to_players_within_position(
                &position,
                floor_level,
                super::EVENT_DELIVERY_RADIUS,
                msg,
                None,
            )
            .await;
        }
        for pid in regen_dirty {
            self.mark_dirty(&pid).await;
        }
    }

    /// Everything that stops when a player drops: movement intent, any
    /// fishing session (a dead angler can't keep a line wet — doc/FISHING.md),
    /// then the XP penalty. One chokepoint so future death sources can't
    /// forget a side effect.
    pub(super) async fn on_player_died(&self, player_id: &PlayerId) {
        self.movement_intents.write().await.remove(player_id);
        self.cancel_concentration_if_active(player_id).await;
        self.apply_player_death_penalty(player_id).await;
    }

    async fn apply_player_death_penalty(&self, player_id: &PlayerId) {
        let (_, old_xp, attributes) = {
            let map = self.player_characters.read().await;
            match map.get(player_id).cloned() {
                Some(entry) => entry,
                None => return,
            }
        };

        let player_name = self.player_name_of(player_id).await;

        let penalty = xp::apply_death_penalty(old_xp);
        let progression_changed =
            penalty.new_xp != penalty.old_xp || penalty.new_level != penalty.old_level;
        if !progression_changed {
            return;
        }

        {
            let mut map = self.player_characters.write().await;
            if let Some(entry) = map.get_mut(player_id) {
                entry.1 = penalty.new_xp;
            }
        }

        let mut current_hp_for_msg = 0;
        let mut max_hp_for_msg = 0;
        let mut level_for_msg = penalty.new_level;

        {
            let mut players = self.players.write().await;
            if let Some(player) = players.get_mut(player_id) {
                player.level = penalty.new_level;

                if penalty.leveled_down {
                    let level_one_floor = match character_hp::level_one_max_hp(
                        character_hp::DEFAULT_CHARACTER_RACE,
                        &player.class,
                        attributes.con,
                    ) {
                        Ok(value) => value,
                        Err(err) => {
                            warn!(
                                "Failed to compute level 1 HP floor for player {}: {}",
                                player_name, err
                            );
                            1
                        }
                    };

                    match character_hp::roll_level_hp_delta(&player.class, attributes.con) {
                        Ok(hp_loss) => {
                            let candidate = i64::from(player.max_health) - i64::from(hp_loss);
                            let bounded = candidate
                                .max(i64::from(level_one_floor))
                                .clamp(1, i64::from(u32::MAX))
                                as u32;

                            if bounded != player.max_health {
                                player.max_health = bounded;
                            }
                        }
                        Err(err) => {
                            warn!(
                                "Failed to roll level-down HP delta for player {}: {}",
                                player_name, err
                            );
                        }
                    }
                }

                if player.health > player.max_health {
                    player.health = player.max_health;
                }

                current_hp_for_msg = player.health;
                max_hp_for_msg = player.max_health;
                level_for_msg = player.level;
            }
        }

        // Mark dirty for periodic batch save
        self.mark_dirty(player_id).await;

        self.send_direct_message(
            player_id,
            ServerMessage::XpGained {
                player_id: *player_id,
                xp_amount: 0,
                xp_lost: penalty.old_xp.saturating_sub(penalty.new_xp),
                total_xp: penalty.new_xp,
                new_level: level_for_msg,
                leveled_up: false,
                max_hp: max_hp_for_msg,
                current_hp: current_hp_for_msg,
            },
        )
        .await;

        info!(
            "Player {} death penalty: XP {} -> {} (penalty {}), level {} -> {}{}",
            player_name,
            penalty.old_xp,
            penalty.new_xp,
            penalty.xp_penalty,
            penalty.old_level,
            level_for_msg,
            if penalty.leveled_down {
                ", level down"
            } else {
                ""
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(x: f32, y: f32, z: f32) -> Position {
        Position { x, y, z }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn weapon_drop_offsets_two_meters_at_angle() {
        let drop_pos = offset_position_at_angle(pos(10.0, 3.0, 20.0), 0.0, 2.0);

        assert_close(drop_pos.x, 12.0);
        assert_close(drop_pos.y, 3.0);
        assert_close(drop_pos.z, 20.0);
    }
}

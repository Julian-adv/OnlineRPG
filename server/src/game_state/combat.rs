use crate::game::{character_hp, combat};

#[derive(Debug, Clone, Copy)]
pub struct EffectiveStats {
    pub guard: i32,
    pub cha: i32,
}

impl From<EffectiveStats> for ServerMessage {
    fn from(s: EffectiveStats) -> Self {
        ServerMessage::EffectiveStatsUpdated {
            guard: s.guard,
            cha: s.cha,
        }
    }
}
use crate::types::{AttackRejectReason, MonsterState, PlayerId, Position, ServerMessage};
use onlinerpg_shared::inventory::{EquipSlot, GroundItem, ItemInstance, PlayerInventory};
use onlinerpg_shared::xp;
use rand::Rng;
use std::f32::consts::TAU;
use std::sync::LazyLock;
use std::time::Duration;
use tracing::{debug, info, warn};

const WEAPON_DROP_OFFSET_METERS: f32 = 2.0;
// Mirrors the web and agent clients' 2m melee reach. This server-side check is
// authoritative: clients may request an attack directly without chasing.
const PLAYER_MELEE_ATTACK_RANGE_METERS: f32 = 2.0;
// The server walks the player at client speed and so trails it by the network
// lag; a blow thrown on arrival must not be refused for that. It belongs to
// every reach, not just the melee one — a bow that stops at its own 10m and
// looses would otherwise be refused for the same trailing metre.
const PLAYER_RANGE_TOLERANCE_METERS: f32 = 1.0;
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

// Hand-measured length of the ChestOpen clip in chest_animated.glb (re-measure
// if the GLB is re-exported): how long treasure-chest loot is withheld before
// bursting out, so the lid is open by the time the drops exist.
pub(super) static CHEST_LOOT_EJECT_DELAY: LazyLock<Duration> =
    LazyLock::new(|| Duration::from_millis(anim_delay_ms("chest_loot_eject")));

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

/// [`pathfinding::attack_line_blocked`] against a wire floor level.
fn wall_between(
    cache: &onlinerpg_shared::pathfinding::PassabilityCache,
    from: Position,
    to: Position,
    floor_level: i8,
) -> bool {
    onlinerpg_shared::pathfinding::attack_line_blocked(
        cache,
        from.x,
        from.z,
        to.x,
        to.z,
        onlinerpg_shared::dungeon::passability_floor_for_level(floor_level),
    )
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
    monster_owner_id: Option<PlayerId>,
    player_name: String,
    player_level: u32,
    /// The swing left melee reach behind, so the target has to be woken
    /// explicitly on a landed hit.
    from_range: bool,
    weapon: EquippedWeapon,
}

/// The wielded main-hand weapon as combat reads it. Resolved once per request,
/// under the one `inventories` read the range gate already needs.
struct EquippedWeapon {
    /// Unarmed falls back to D&D 5e improvised 1d2.
    dice: String,
    /// An enchanted weapon (+N) adds its enchant to attack and damage rolls.
    enchant: i32,
    /// Reach in meters, or `None` for melee.
    range: Option<f32>,
    /// Ability the hit and damage bonus read, or `None` to keep STR.
    ability: Option<crate::item_defs::Ability>,
}

impl Default for EquippedWeapon {
    fn default() -> Self {
        Self {
            dice: "1d2".to_string(),
            enchant: 0,
            range: None,
            ability: None,
        }
    }
}

struct PlayerAttackTarget {
    state: MonsterState,
    monster_type: String,
    position: Position,
    floor_level: i8,
    level_override: Option<u8>,
    owner_id: Option<PlayerId>,
}

impl super::GameState {
    /// Put a kill's loot — the weapon drop and any rare world drops — on the
    /// ground once the killing blow lands. The wait lives server-side: a
    /// ground item is pickable the moment it exists, so a client that skips
    /// the animation must not reach it early.
    pub(super) fn spawn_kill_loot_after_impact(
        &self,
        weapon_drop: Option<GroundItem>,
        monster_drops: Vec<String>,
        origin: Position,
        floor_level: i8,
        monster_level: Option<u8>,
    ) {
        let game_state = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(*PLAYER_ATTACK_IMPACT_DELAY).await;
            if let Some(item) = weapon_drop {
                game_state.spawn_ground_item(item).await;
            }
            game_state
                .spawn_world_drops(origin, floor_level, monster_level, monster_drops)
                .await;
        });
    }

    async fn claim_player_attack_window(&self, player_id: &PlayerId) -> bool {
        let now = Self::now_ms();
        // Hunger only lengthens the interval, so an attack still inside the
        // base window never needs the hunger lookup — spam-clicks stay cheap.
        if self
            .last_player_attacks
            .read()
            .await
            .get(player_id)
            .is_some_and(|last| now.saturating_sub(*last) < *PLAYER_ATTACK_INTERVAL_MS)
        {
            return false;
        }
        let attack_mult = self.hunger_attack_mult(player_id).await.max(f32::EPSILON);
        let required_interval = (*PLAYER_ATTACK_INTERVAL_MS as f32 / attack_mult).ceil() as u64;
        let mut last_attacks = self.last_player_attacks.write().await;
        let last = last_attacks.entry(*player_id).or_insert(0);
        if now.saturating_sub(*last) < required_interval {
            return false;
        }
        *last = now;
        true
    }

    /// A player's worn gear paired with its defs — the single place that
    /// resolves equipped items to their definitions.
    fn equipped_pairs<'a>(
        &'a self,
        inv: &'a PlayerInventory,
    ) -> impl Iterator<Item = (&'a ItemInstance, &'a crate::item_defs::ItemDefinition)> {
        inv.equipped
            .values()
            .filter_map(|item| Some((item, self.item_defs.get(&item.item_def_id)?)))
    }

    /// The defs behind a player's worn gear, for callers that don't need the
    /// instance (and so its enchant).
    pub(super) fn equipped_defs<'a>(
        &'a self,
        inv: &'a PlayerInventory,
    ) -> impl Iterator<Item = &'a crate::item_defs::ItemDefinition> {
        self.equipped_pairs(inv).map(|(_, def)| def)
    }

    /// Sum of one def stat over every equipped item.
    pub(super) fn equipped_bonus(
        &self,
        inv: &PlayerInventory,
        stat: fn(&crate::item_defs::ItemDefinition) -> Option<i32>,
    ) -> i32 {
        self.equipped_defs(inv).filter_map(stat).sum()
    }

    /// Equipped `guard` plus worn-armor enchant levels. A weapon's enchant
    /// stays on attack and damage rolls and never protects.
    pub(super) fn equipped_guard(&self, inv: &PlayerInventory) -> i32 {
        self.equipped_pairs(inv)
            .map(|(item, def)| {
                def.guard.unwrap_or(0) + if def.is_armor() { item.enchant } else { 0 }
            })
            .sum()
    }

    /// A player's effective stats (base attribute plus equipped-gear bonuses),
    /// read under one pass of each lock. Guard is the target number an
    /// attacker must beat; CHA drives haggling bands. Reported to the client
    /// so it never recomputes the formula itself.
    pub async fn effective_stats(&self, player_id: &PlayerId) -> EffectiveStats {
        let (guard, cha) = {
            let chars = self.player_characters.read().await;
            chars
                .get(player_id)
                .map(|(_, _, a)| (i32::from(a.guard), i32::from(a.cha)))
                .unwrap_or((10, 10))
        };
        let inventories = self.inventories.read().await;
        let (guard_bonus, cha_bonus) = inventories
            .get(player_id)
            .map(|inv| {
                (
                    self.equipped_guard(inv),
                    self.equipped_bonus(inv, |d| d.cha_bonus()),
                )
            })
            .unwrap_or((0, 0));
        EffectiveStats {
            guard: guard + guard_bonus,
            cha: cha + cha_bonus,
        }
    }

    pub async fn effective_guard(&self, player_id: &PlayerId) -> i32 {
        self.effective_stats(player_id).await.guard
    }

    pub(super) async fn effective_cha(&self, player_id: &PlayerId) -> i32 {
        self.effective_stats(player_id).await.cha
    }

    async fn equipped_weapon(&self, player_id: &PlayerId) -> EquippedWeapon {
        let inventories = self.inventories.read().await;
        inventories
            .get(player_id)
            .and_then(|inv| inv.equipped.get(&EquipSlot::MainHand))
            .and_then(|item| {
                self.item_defs.get(&item.item_def_id).and_then(|def| {
                    def.damage_dice().map(|dice| EquippedWeapon {
                        dice: dice.to_string(),
                        enchant: item.enchant,
                        range: def.weapon_range(),
                        ability: def.ranged_ability(),
                    })
                })
            })
            .unwrap_or_default()
    }

    /// Tell a client-owned monster's controller to aggro onto the attacker.
    /// Server-side brains wake through `brain_hit` instead.
    async fn notify_monster_provoked(
        &self,
        monster_id: &str,
        player_id: &PlayerId,
        owner_id: Option<PlayerId>,
    ) {
        let Some(owner_id) = owner_id else { return };
        self.send_direct_message(
            &owner_id,
            ServerMessage::MonsterProvoked {
                player_id: *player_id,
                monster_id: monster_id.to_string(),
            },
        )
        .await;
    }

    /// Runs every gate on a `PlayerAttack` request. `Err` is the coarse reason
    /// acked back to the attacker plus the gate detail the single call site
    /// logs, so a new gate can never silently drop a request. Side effect: an
    /// out-of-range swing within provoke range still aggros the monster onto
    /// the attacker.
    async fn validate_player_attack(
        &self,
        player_id: &PlayerId,
        monster_id: &str,
    ) -> Result<PlayerAttackContext, (AttackRejectReason, String)> {
        let monster = {
            let monsters = self.monsters.read().await;
            monsters.get(monster_id).map(|monster| PlayerAttackTarget {
                state: monster.state,
                monster_type: monster.monster_type.clone(),
                position: monster.position,
                floor_level: monster.floor_level,
                level_override: monster.level_override,
                owner_id: monster.owner_id,
            })
        };
        let Some(monster) = monster else {
            return Err((
                AttackRejectReason::InvalidTarget,
                "absent from registry".into(),
            ));
        };
        if monster.state == MonsterState::Dead {
            return Err((
                AttackRejectReason::InvalidTarget,
                format!(
                    "corpse ({}, floor {})",
                    monster.monster_type, monster.floor_level
                ),
            ));
        }
        let now = Self::now_ms();
        let player_snapshot = {
            let players = self.players.read().await;
            players
                .get(player_id)
                .filter(|p| p.is_ready(now))
                .map(|p| (p.name.clone(), p.level, p.floor_level, p.position, p.health))
        };
        let Some((player_name, player_level, player_floor, player_position, player_health)) =
            player_snapshot
        else {
            return Err((
                AttackRejectReason::NotInGame,
                "not in game or still loading".into(),
            ));
        };

        // A dead attacker deals no damage. The monster-attack path already
        // gates on the target's HP; mirror it here so a 0-HP player can't
        // keep swinging while awaiting respawn.
        if player_health == 0 {
            return Err((AttackRejectReason::AttackerDead, "attacker hp 0".into()));
        }
        // Delivery filtering keeps a player from ever learning about
        // monsters on another floor, but gate here too so a stale monster
        // id can't drive a cross-floor hit (the original bug: a surface
        // guard striking a monster on the dungeon floor beneath it).
        let melee_range = PLAYER_MELEE_ATTACK_RANGE_METERS + PLAYER_RANGE_TOLERANCE_METERS;
        // A weapon that declares a `range` (items.csv) shoots that far; the
        // clients gate on the same column, so the two never drift. The lag
        // allowance rides on top of whichever reach is in play rather than
        // being folded into the melee one, which `max` would swallow whole.
        let weapon = self.equipped_weapon(player_id).await;
        let attack_range = PLAYER_MELEE_ATTACK_RANGE_METERS.max(weapon.range.unwrap_or(0.0))
            + PLAYER_RANGE_TOLERANCE_METERS;
        let reach = |target: Position| {
            reachable_dist_sq(player_position, player_floor, target, monster.floor_level)
        };
        let unreachable_detail = |label: &str, target: Position| {
            format!(
                "{} alive but unreachable ({label}): player floor {player_floor} at ({:.1},{:.1}), monster floor {} at ({:.1},{:.1})",
                monster.monster_type, player_position.x, player_position.z, monster.floor_level, target.x, target.z
            )
        };
        let Some(mut distance_sq) = reach(monster.position) else {
            return Err((
                AttackRejectReason::InvalidTarget,
                unreachable_detail("registry", monster.position),
            ));
        };
        // The registry trails a running monster by up to a sync interval, so
        // a swing it refuses is re-judged where the brain has the monster.
        let mut gate_position = monster.position;
        if distance_sq > attack_range.powi(2) {
            if let Some(now) = self.brain_position_now(monster_id).await {
                let Some(d) = reach(now) else {
                    return Err((
                        AttackRejectReason::InvalidTarget,
                        unreachable_detail("brain", now),
                    ));
                };
                gate_position = now;
                distance_sq = d;
            }
        }
        let walled_off = || {
            wall_between(
                &self.passability_read(),
                player_position,
                gate_position,
                player_floor,
            )
        };
        if distance_sq > attack_range.powi(2) {
            // A swing at a monster behind a shut door must not reach it as
            // aggro either.
            if distance_sq <= PLAYER_ATTACK_PROVOKE_RANGE_METERS.powi(2) && !walled_off() {
                if self.server_monster_ai() {
                    self.brain_hit(monster_id, player_id, false, 0).await;
                } else {
                    self.notify_monster_provoked(monster_id, player_id, monster.owner_id)
                        .await;
                }
            }
            return Err((
                AttackRejectReason::OutOfRange,
                format!("dist {:.1}m > {:.1}m", distance_sq.sqrt(), attack_range),
            ));
        }
        if walled_off() {
            return Err((AttackRejectReason::OutOfRange, "walled off".into()));
        }

        Ok(PlayerAttackContext {
            monster_type: monster.monster_type,
            monster_position: monster.position,
            monster_floor_level: monster.floor_level,
            monster_level_override: monster.level_override,
            monster_owner_id: monster.owner_id,
            player_name,
            player_level,
            from_range: distance_sq > melee_range.powi(2),
            weapon,
        })
    }

    #[cfg(test)]
    pub async fn broadcast_player_attack(&self, player_id: &PlayerId, monster_id: String) {
        self.player_attack(player_id, monster_id, None).await;
    }

    /// `auth` persists boss-kill titles; tests pass None and keep them in
    /// memory.
    pub async fn player_attack(
        &self,
        player_id: &PlayerId,
        monster_id: String,
        auth: Option<&crate::auth::AuthService>,
    ) {
        let PlayerAttackContext {
            monster_type,
            monster_position,
            monster_floor_level,
            monster_level_override,
            monster_owner_id,
            player_name,
            player_level,
            from_range,
            weapon,
        } = match self.validate_player_attack(player_id, &monster_id).await {
            Ok(ctx) => ctx,
            Err((reason, detail)) => {
                warn!(
                    "Rejected player attack: {} -> {} ({:?}: {})",
                    player_id, monster_id, reason, detail
                );
                self.send_direct_message(
                    player_id,
                    ServerMessage::PlayerAttackRejected { monster_id, reason },
                )
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

        // A ranged weapon rolls on the ability it declares (DEX for the bow);
        // everything else keeps STR.
        let ability_mod = {
            let chars = self.player_characters.read().await;
            chars
                .get(player_id)
                .map(|(_, _, attrs)| {
                    combat::ability_modifier(match weapon.ability {
                        Some(ability) => ability.score(attrs),
                        None => attrs.r#str,
                    })
                })
                .unwrap_or(0)
        };

        let hit_mod = self.hunger_hit_mod(player_id).await;

        let (result_hit, result_roll, result_damage) = {
            let def = self.monster_defs.get(&monster_type);
            let target_guard = def.map(|d| i32::from(d.guard)).unwrap_or(10);
            let attack_bonus =
                combat::level_attack_bonus(player_level) + ability_mod + weapon.enchant + hit_mod;
            let result = combat::roll_attack(
                attack_bonus,
                target_guard,
                &weapon.dice,
                ability_mod + weapon.enchant,
            );
            (result.hit, result.roll, result.damage)
        };

        debug!(
            "Dice roll: {}, Hit: {}, Damage: {}",
            result_roll, result_hit, result_damage
        );

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
                damage: result_damage,
            },
            None,
        )
        .await;

        if !result_hit {
            self.brain_hit(&monster_id, player_id, false, 0).await;
        } else {
            // Damage and the kill claim share one guard, so a second attacker
            // landing at the same instant sees Dead and stops.
            let mut dealt = 0;
            let is_dead = {
                let mut monsters = self.monsters.write().await;
                let mut died = false;

                if let Some(monster) = monsters.get_mut(&monster_id) {
                    if monster.state == MonsterState::Dead {
                        return; // Already dead
                    }

                    dealt = result_damage.min(monster.health);
                    monster.health = monster.health.saturating_sub(result_damage);
                    debug!(
                        "Monster {} HP: {}/{}",
                        monster_id, monster.health, monster.max_health
                    );

                    died = monster.health == 0;
                }
                if died {
                    // Through the registry so the kill frees its spawn slot now,
                    // not when the corpse is swept 60s later.
                    monsters.mark_dead(&monster_id);
                }
                died
            };
            if monster_floor_level < 0 {
                self.record_boss_damage(&monster_id, player_id, dealt).await;
            }
            if !is_dead {
                self.brain_hit(&monster_id, player_id, true, result_damage)
                    .await;
                // A client-owned brain only applies the hit at its own impact
                // frame; a shot landed from outside melee reach must aggro now.
                if from_range && !self.server_monster_ai() {
                    self.notify_monster_provoked(&monster_id, player_id, monster_owner_id)
                        .await;
                }
            } else {
                // Loot lands where the monster fell, which is not where the
                // registry has it: that only advances on the brain's tick, and
                // a bow kills a monster mid-charge, metres along from its last
                // sync. Read the brain before `brain_death` drops it.
                let corpse_position = self
                    .brain_position_now(&monster_id)
                    .await
                    .unwrap_or(monster_position);
                self.brain_death(&monster_id).await;
                let def = self.monster_defs.get(&monster_type);
                // Depth-scaled dungeon monsters count as their effective level.
                let effective_level = monster_level_override.or(def.map(|d| d.level));
                let dropped_weapon_item_def_id = def
                    .filter(|def| {
                        def.weapon_drop_chance >= 1.0
                            || rand::thread_rng().gen::<f32>() < def.weapon_drop_chance
                    })
                    .and_then(|def| def.weapon.as_deref())
                    .and_then(|weapon| self.item_defs.item_def_id_for_weapon_ref(weapon));

                info!(
                    "Player {} killed {} (lvl {}) at ({:.1},{:.1}) {}; weapon drop: {}",
                    player_name,
                    monster_type,
                    effective_level.unwrap_or(0),
                    corpse_position.x,
                    corpse_position.z,
                    crate::dungeon_defs::place_label(&corpse_position, monster_floor_level),
                    dropped_weapon_item_def_id.as_deref().unwrap_or("none")
                );
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
                    // floor: pickup is a pure proximity check, so an item
                    // behind a wall would be lost.
                    let drop_position = self
                        .loot_drop_position(
                            corpse_position,
                            monster_floor_level,
                            dropped_weapon_position(corpse_position),
                        )
                        .await;
                    Some(GroundItem {
                        instance_id,
                        item_def_id,
                        position: drop_position,
                        floor_level: monster_floor_level,
                        quantity: 1,
                        enchant: 0,
                        dropped_by: None,
                        cape_color: None,
                        cape_texture: None,
                    })
                } else {
                    None
                };
                let mut monster_drops = def.map_or_else(Vec::new, |def| {
                    crate::world_drop_defs::roll_independent(
                        def.drop_entries().iter().map(|(id, c)| (id.as_str(), *c)),
                        &mut rand::thread_rng(),
                    )
                });
                monster_drops.extend(
                    self.on_dungeon_monster_dead(&monster_id, player_id, auth)
                        .await,
                );
                // All kill loot waits for the blow to land.
                self.spawn_kill_loot_after_impact(
                    weapon_drop,
                    monster_drops,
                    corpse_position,
                    monster_floor_level,
                    effective_level,
                );

                self.drain_hunger_for_kill(player_id).await;
                if let (Some(def), Some(effective_level)) = (def, effective_level) {
                    let base_xp = xp::monster_xp(effective_level, def.guard);
                    let sharers = self
                        .party_members_sharing_kill(
                            player_id,
                            monster_position,
                            monster_floor_level,
                        )
                        .await;
                    let share = xp::party_xp_share(base_xp, sharers.len() as u32 + 1);
                    let mut recipients = Vec::with_capacity(sharers.len() + 1);
                    recipients.push(*player_id);
                    recipients.extend(sharers);
                    self.grant_monster_kill_xp(&recipients, share, &monster_id)
                        .await;
                }

                // Schedule removal after 60 seconds. Through despawn_monsters
                // so the removal reaches the corpse's owner directly even when
                // it has wandered out of range — the radius fanout alone left
                // a ghost corpse on the owner's client.
                let game_state = self.clone();
                let id_to_remove = monster_id.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                    let still_dead = game_state
                        .monsters
                        .read()
                        .await
                        .get(&id_to_remove)
                        .is_some_and(|monster| monster.state == MonsterState::Dead);
                    if still_dead {
                        debug!("Monster {} removed after 60s corpse time", id_to_remove);
                        game_state.despawn_monsters(vec![id_to_remove]).await;
                    }
                });
            }
        }
    }

    /// Credit every sharer of a monster kill: cumulative XP, any level-ups
    /// (HP rolls plus the full heal), dirty marks, and the direct XpGained
    /// notice. A zero share (tiny monster split) sends nothing. Batched over
    /// the whole party so one kill takes each lock once.
    async fn grant_monster_kill_xp(
        &self,
        recipients: &[PlayerId],
        xp_amount: u32,
        monster_id: &str,
    ) {
        if xp_amount == 0 {
            return;
        }
        // Read-modify-write under one lock: a member can receive two kills'
        // shares concurrently, and a split read/write would lose one.
        let mut grants = Vec::with_capacity(recipients.len());
        {
            let mut map = self.player_characters.write().await;
            for player_id in recipients {
                if let Some(entry) = map.get_mut(player_id) {
                    let old_xp = entry.1;
                    entry.1 += u64::from(xp_amount);
                    grants.push((*player_id, old_xp, entry.1, entry.2.clone()));
                }
            }
        }
        if grants.is_empty() {
            return;
        }

        // One `players` guard for the level-up rolls and the HP the notices
        // carry. Taken after `player_characters` is released — the regen tick
        // takes the two in the opposite order.
        let mut notices = Vec::with_capacity(grants.len());
        let mut leveled = Vec::new();
        {
            let mut players = self.players.write().await;
            for (player_id, old_xp, new_xp, attributes) in &grants {
                let old_level = xp::level_from_xp(*old_xp);
                let new_level = xp::level_from_xp(*new_xp);
                let leveled_up = new_level > old_level;
                let Some(p) = players.get_mut(player_id) else {
                    continue;
                };
                if leveled_up {
                    // Concurrent grants may reach this lock out of XP order;
                    // the HP rolls cover disjoint level spans either way, but
                    // the level itself must never move backwards. This guards
                    // authoritative state only — the client clamps the notices,
                    // which can be sent out of order after the lock is dropped.
                    if new_level > p.level {
                        p.level = new_level;
                    }
                    let mut updated_max_hp = p.max_health;
                    for _ in 0..new_level.saturating_sub(old_level) {
                        match character_hp::level_up_max_hp(
                            updated_max_hp,
                            &p.class,
                            attributes.con,
                        ) {
                            Ok(next_max_hp) => updated_max_hp = next_max_hp,
                            Err(err) => {
                                warn!("Failed to roll level-up HP for player {}: {}", p.name, err);
                                break;
                            }
                        }
                    }
                    p.max_health = updated_max_hp;
                    // Level-up always fully restores current HP to max HP.
                    p.health = p.max_health;
                    leveled.push(*player_id);
                }
                notices.push((
                    *player_id,
                    p.name.clone(),
                    *new_xp,
                    new_level,
                    leveled_up,
                    p.max_health,
                    p.health,
                ));
            }
        }

        self.dirty_players
            .write()
            .await
            .extend(grants.iter().map(|(id, ..)| *id));
        if !leveled.is_empty() {
            self.party_vitals_dirty.write().await.extend(leveled);
        }

        for (player_id, name, new_xp, new_level, leveled_up, max_hp, current_hp) in notices {
            self.send_direct_message(
                &player_id,
                ServerMessage::XpGained {
                    player_id,
                    xp_amount,
                    xp_lost: 0,
                    total_xp: new_xp,
                    new_level,
                    leveled_up,
                    max_hp,
                    current_hp,
                    monster_id: Some(monster_id.to_string()),
                },
            )
            .await;

            if leveled_up {
                info!("Player {} reached level {}", name, new_level);
            } else {
                debug!(
                    "Player {} gained {} XP (total: {}, level: {})",
                    name, xp_amount, new_xp, new_level
                );
            }
        }
    }

    /// A client's swing request. With server brains on it is ignored: the
    /// registry still files a cap owner, so the ownership gate alone would
    /// let that client aim the monster.
    pub async fn broadcast_monster_attack(
        &self,
        attacker_player_id: &PlayerId,
        monster_id: &str,
        target_player_id: &PlayerId,
    ) {
        if self.server_monster_ai() {
            return;
        }
        self.monster_attack(Some(attacker_player_id), monster_id, target_player_id)
            .await;
    }

    /// `attacker` is the owning client, or `None` for the server's own brain.
    /// Cooldown, reach and wall checks apply to both.
    pub(super) async fn monster_attack(
        &self,
        attacker: Option<&PlayerId>,
        monster_id: &str,
        target_player_id: &PlayerId,
    ) {
        struct MonsterAttackSnapshot {
            attack_bonus: i32,
            damage_roll: String,
            weapon_damage_roll: Option<String>,
            position: Position,
            floor_level: i8,
            attack_range: f32,
            hit_debuff: Option<String>,
            monster_type: String,
        }
        let now = Self::now_ms();
        let mut monster_data = None;

        {
            let mut monsters = self.monsters.write().await;
            if let Some(monster) = monsters.get_mut(monster_id) {
                let controllable = match attacker {
                    Some(attacker) => monster.is_controllable_by(attacker),
                    None => monster.state != MonsterState::Dead,
                };
                if controllable {
                    let def = self.monster_defs.get(&monster.monster_type);
                    let attack_cooldown_ms =
                        def.map(|d| u64::from(d.attack_cooldown)).unwrap_or(1500);

                    if now.saturating_sub(monster.last_attack_at) >= attack_cooldown_ms {
                        monster.last_attack_at = now;
                        let weapon_damage_roll = def
                            .and_then(|d| d.weapon.as_deref())
                            .and_then(|weapon| self.item_defs.damage_dice_for_weapon_model(weapon));
                        // Depth-scaled dungeon monsters attack at their
                        // effective level (bonus + damage dice).
                        let (attack_bonus, damage_roll) = match monster.level_override {
                            Some(level) => (
                                def.map_or_else(
                                    || combat::monster_attack_bonus(level),
                                    |d| d.attack_bonus_at(level),
                                ),
                                combat::monster_damage_roll_for_level(level).to_string(),
                            ),
                            None => (
                                def.map(|d| d.attack_bonus())
                                    .unwrap_or_else(|| combat::monster_attack_bonus(1)),
                                def.map(|d| d.damage_roll())
                                    .unwrap_or_else(|| "1d6".to_string()),
                            ),
                        };
                        monster_data = Some(MonsterAttackSnapshot {
                            attack_bonus,
                            damage_roll,
                            weapon_damage_roll,
                            position: monster.position,
                            floor_level: monster.floor_level,
                            attack_range: def
                                .map(|d| d.attack_range)
                                .unwrap_or(onlinerpg_shared::monster_ai::DEFAULT_ATTACK_RANGE),
                            hit_debuff: def.and_then(|d| d.hit_debuff.clone()),
                            monster_type: monster.monster_type.clone(),
                        });
                    }
                }
            }
        }

        let Some(MonsterAttackSnapshot {
            attack_bonus,
            damage_roll,
            weapon_damage_roll,
            position: monster_position,
            floor_level: monster_floor_level,
            attack_range: monster_attack_range,
            hit_debuff,
            monster_type,
        }) = monster_data
        else {
            return;
        };

        // 2. Check if target player exists and can take damage
        let (target_player_name, target_position, target_floor_level);
        {
            let players = self.players.read().await;
            match players.get(target_player_id) {
                Some(player) if player.is_damageable(now) => {
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
        if wall_between(
            &self.passability_read(),
            monster_position,
            target_position,
            monster_floor_level,
        ) {
            debug!(
                "Rejected monster attack through a wall: monster {} -> player {}",
                monster_id, target_player_name
            );
            return;
        }
        let target_guard = self.effective_guard(target_player_id).await;
        let place_bonus =
            combat::place_attack_bonus(monster_floor_level, self.town_distance(&monster_position));

        let result = combat::roll_attack_with_extra_damage_roll(
            attack_bonus + place_bonus,
            target_guard,
            &damage_roll,
            weapon_damage_roll.as_deref(),
            0,
        );

        debug!(
            "Monster {} attacks player {}: Roll {}, Hit: {}, Damage: {}",
            monster_id, target_player_name, result.roll, result.hit, result.damage
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
                    player.health = player.health.saturating_sub(result.damage);
                    if player.health == 0 {
                        did_die = true;
                    }
                }
                current_health = player.health;
                target_loc = Some((player.position, player.floor_level));
            }
        }

        if result.hit {
            self.cancel_live_instrument_if_active(target_player_id)
                .await;
            self.cancel_food_regeneration(target_player_id).await;
            self.mark_dirty(target_player_id).await;
            self.mark_party_vitals_dirty(target_player_id).await;
        }

        // Send attack result after server-side HP update.
        let attack_msg = ServerMessage::MonsterAttackedPlayer {
            monster_id: monster_id.to_string(),
            player_id: *target_player_id,
            hit: result.hit,
            roll: result.roll,
            damage: result.damage,
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

        if result.hit && !did_die {
            if let Some(debuff_id) = hit_debuff {
                self.inflict_debuff(target_player_id, &debuff_id, None)
                    .await;
            }
        }

        if let (true, Some((target_position, target_floor))) = (did_die, target_loc) {
            self.announce_player_death(
                target_player_id,
                target_position,
                target_floor,
                &monster_type,
            )
            .await;
        }
    }

    /// Run the death chokepoint and tell the area.
    pub(super) async fn announce_player_death(
        &self,
        player_id: &PlayerId,
        position: Position,
        floor_level: i8,
        cause: &str,
    ) {
        self.on_player_died(player_id, cause).await;
        self.send_direct_message_to_players_within_position(
            &position,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::PlayerDead {
                player_id: *player_id,
            },
            None,
        )
        .await;
    }

    pub async fn tick_regeneration(&self) {
        let mut updates = Vec::new();

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

        let candidates: Vec<PlayerId> = updates.iter().map(|(pid, _)| *pid).collect();
        let include_hungry = self
            .regen_ticks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % 2
            == 1;
        let regen_ready = self.hunger_regen_ready(&candidates, include_hungry).await;
        updates.retain(|(pid, _)| regen_ready.contains(pid));
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
        if !regen_dirty.is_empty() {
            self.dirty_players.write().await.extend(regen_dirty.iter());
            self.party_vitals_dirty.write().await.extend(regen_dirty);
        }
    }

    /// Everything that stops when a player drops: movement intent, any
    /// fishing session (a dead angler can't keep a line wet — doc/FISHING.md),
    /// then the XP penalty. One chokepoint so future death sources can't
    /// forget a side effect.
    pub(super) async fn on_player_died(&self, player_id: &PlayerId, cause: &str) {
        if let Some((position, _, floor_level, name)) = self.player_pose(player_id).await {
            let place = crate::dungeon_defs::place_label(&position, floor_level);
            info!(
                "Player {name} died to {cause} at ({:.1},{:.1}) {place}",
                position.x, position.z
            );
        }
        self.movement_intents.write().await.remove(player_id);
        self.cancel_concentration_if_active(player_id).await;
        self.cancel_food_regeneration(player_id).await;
        self.clear_debuffs(player_id).await;
        self.drop_player_trade(player_id, "They were defeated.")
            .await;
        self.apply_player_death_penalty(player_id).await;
    }

    async fn apply_player_death_penalty(&self, player_id: &PlayerId) {
        // Read-modify-write under one lock, like the kill-XP grant: a shared
        // kill can land while the penalty is computed, and a split read/write
        // would drop it.
        let applied = {
            let mut map = self.player_characters.write().await;
            map.get_mut(player_id).and_then(|entry| {
                let penalty = xp::apply_death_penalty(entry.1);
                let progression_changed =
                    penalty.new_xp != penalty.old_xp || penalty.new_level != penalty.old_level;
                if !progression_changed {
                    return None;
                }
                entry.1 = penalty.new_xp;
                Some((penalty, entry.2.clone()))
            })
        };
        let Some((penalty, attributes)) = applied else {
            return;
        };

        let player_name = self.player_name_of(player_id).await;

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
        self.mark_party_vitals_dirty(player_id).await;

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
                monster_id: None,
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

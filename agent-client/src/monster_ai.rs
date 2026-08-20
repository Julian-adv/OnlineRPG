//! Monster AI adapter — delegates to `onlinerpg_shared::monster_ai`.

use onlinerpg_shared::dungeon::passability_floor_for_level;
use onlinerpg_shared::monster_ai::{
    self, AiCommand, BehaviorTree, CachePathProvider, MonsterBrain, NearbyPlayer,
    AGGRESSIVE_BEHAVIOR, DEFAULT_ATTACK_COOLDOWN_MS, DEFAULT_ATTACK_RANGE, DEFAULT_BEHAVIOR,
    DEFAULT_CHASE_RANGE, DEFAULT_RUN_SPEED, DEFAULT_WALK_SPEED,
};
use onlinerpg_shared::pathfinding::PassabilityCache;
use onlinerpg_shared::{ClientMessage, Monster, Player, PlayerId, Position};
use std::collections::HashMap;
use tracing::info;

/// Manages all monster brains assigned to this agent-client.
pub struct MonsterAiManager {
    brains: HashMap<String, MonsterBrain>,
    behavior_trees: HashMap<String, BehaviorTree>,
    /// Maps monster_type -> behavior tree name.
    type_to_behavior: HashMap<String, String>,
    type_to_movement: HashMap<String, MonsterMovement>,
}

#[derive(Debug, Clone, Copy)]
pub struct MonsterMovement {
    pub walk_speed: f32,
    pub run_speed: f32,
    pub attack_range: f32,
    pub chase_range: f32,
    pub attack_cooldown_ms: f32,
}

impl Default for MonsterMovement {
    fn default() -> Self {
        Self {
            walk_speed: DEFAULT_WALK_SPEED,
            run_speed: DEFAULT_RUN_SPEED,
            attack_range: DEFAULT_ATTACK_RANGE,
            chase_range: DEFAULT_CHASE_RANGE,
            attack_cooldown_ms: DEFAULT_ATTACK_COOLDOWN_MS,
        }
    }
}

impl MonsterAiManager {
    pub fn new() -> Self {
        Self {
            brains: HashMap::new(),
            behavior_trees: HashMap::new(),
            type_to_behavior: HashMap::new(),
            type_to_movement: HashMap::new(),
        }
    }

    /// Load behavior trees from JSON (data-src/behavior_trees.json).
    pub fn load_behavior_trees_from_json(json: &str) -> HashMap<String, BehaviorTree> {
        monster_ai::load_behavior_trees(json).unwrap_or_default()
    }

    /// Load per-type behavior names and movement/combat constants from generated monsters.json.
    pub fn load_monster_data(
        monsters_json: &str,
    ) -> (HashMap<String, String>, HashMap<String, MonsterMovement>) {
        #[derive(serde::Deserialize)]
        struct RawMonster {
            #[serde(default = "default_behavior")]
            behavior: String,
            #[serde(rename = "walkSpeed", default = "default_walk_speed")]
            walk_speed: f32,
            #[serde(rename = "runSpeed", default = "default_run_speed")]
            run_speed: f32,
            #[serde(rename = "attackRange", default = "default_attack_range")]
            attack_range: f32,
            #[serde(rename = "chaseRange", default = "default_chase_range")]
            chase_range: f32,
            #[serde(rename = "attackCooldown", default = "default_attack_cooldown_ms")]
            attack_cooldown_ms: f32,
        }
        fn default_behavior() -> String {
            DEFAULT_BEHAVIOR.to_string()
        }
        fn default_walk_speed() -> f32 {
            MonsterMovement::default().walk_speed
        }
        fn default_run_speed() -> f32 {
            MonsterMovement::default().run_speed
        }
        fn default_attack_range() -> f32 {
            MonsterMovement::default().attack_range
        }
        fn default_chase_range() -> f32 {
            MonsterMovement::default().chase_range
        }
        fn default_attack_cooldown_ms() -> f32 {
            MonsterMovement::default().attack_cooldown_ms
        }

        let raw: HashMap<String, RawMonster> =
            serde_json::from_str(monsters_json).unwrap_or_default();
        let mut type_to_behavior = HashMap::with_capacity(raw.len());
        let mut type_to_movement = HashMap::with_capacity(raw.len());
        for (id, r) in raw {
            type_to_behavior.insert(id.clone(), r.behavior);
            type_to_movement.insert(
                id,
                MonsterMovement {
                    walk_speed: r.walk_speed,
                    run_speed: r.run_speed,
                    attack_range: r.attack_range,
                    chase_range: r.chase_range,
                    attack_cooldown_ms: r.attack_cooldown_ms,
                },
            );
        }
        (type_to_behavior, type_to_movement)
    }

    pub fn set_behavior_trees(&mut self, behavior_trees: HashMap<String, BehaviorTree>) {
        self.behavior_trees = behavior_trees;
    }

    pub fn set_type_mapping(&mut self, mapping: HashMap<String, String>) {
        self.type_to_behavior = mapping;
    }

    pub fn set_movement_speeds(&mut self, movement: HashMap<String, MonsterMovement>) {
        self.type_to_movement = movement;
    }

    /// Resolve the behavior tree name for a monster type, falling back to "default".
    fn behavior_for(&self, monster_type: &str) -> String {
        self.type_to_behavior
            .get(monster_type)
            .cloned()
            .unwrap_or_else(|| DEFAULT_BEHAVIOR.to_string())
    }

    /// Register a newly assigned monster.
    pub fn add_monster(&mut self, monster: &Monster) {
        info!(
            "Monster AI: managing {} (type={}, aggressive={})",
            monster.id, monster.monster_type, monster.aggressive
        );
        // Aggressive (선공형) spawns acquire targets on sight regardless of the
        // type's default timid/brave behavior.
        let behavior = if monster.aggressive {
            AGGRESSIVE_BEHAVIOR.to_string()
        } else {
            self.behavior_for(&monster.monster_type)
        };
        let movement = self
            .type_to_movement
            .get(&monster.monster_type)
            .copied()
            .unwrap_or_default();
        let mut brain = MonsterBrain::new(
            monster.id.clone(),
            monster.monster_type.clone(),
            behavior,
            monster.position,
            monster.health,
            monster.max_health,
            movement.walk_speed,
            movement.run_speed,
            movement.attack_range,
            movement.chase_range,
            movement.attack_cooldown_ms,
        );
        // A dungeon monster paths on its own floor's grid; left at 0 it would
        // path against the surface and walk through every wall around it.
        brain.path_floor = passability_floor_for_level(monster.floor_level);
        self.brains.insert(monster.id.clone(), brain);
    }

    /// Remove a monster (died or removed).
    pub fn remove_monster(&mut self, monster_id: &str) {
        if self.brains.remove(monster_id).is_some() {
            info!("Monster AI: stopped managing {}", monster_id);
        }
    }

    /// Notify that a monster was hit by a player.
    pub fn handle_monster_hit(
        &mut self,
        monster_id: &str,
        attacker_id: &PlayerId,
        hit: bool,
        damage: u32,
        _passability_cache: &PassabilityCache,
    ) -> Vec<ClientMessage> {
        let Some(brain) = self.brains.get_mut(monster_id) else {
            return vec![];
        };
        let cmds = brain.handle_hit_with_behavior_tree(attacker_id, hit, damage);
        cmds.into_iter().map(command_to_client_msg).collect()
    }

    /// Re-sync a managed monster to the server's authoritative position.
    pub fn apply_authoritative_position(&mut self, monster_id: &str, position: Position) {
        if let Some(brain) = self.brains.get_mut(monster_id) {
            brain.apply_authoritative_position(position);
        }
    }

    /// Notify that a monster died.
    pub fn handle_monster_dead(&mut self, monster_id: &str) {
        if let Some(brain) = self.brains.get_mut(monster_id) {
            brain.handle_death();
        }
    }

    /// Tick all managed monster brains. Returns commands to send.
    ///
    /// `self_player` is required: the server's `nearby_players` never includes
    /// us, so without it our own monsters would never target the NPC we drive.
    /// `self_pass_floor` is our passability floor; a brain on another floor
    /// must not see us, or it would chase its owner through the floor.
    pub fn tick_all(
        &mut self,
        delta_ms: f32,
        nearby_players: &HashMap<PlayerId, Player>,
        self_player: Option<&Player>,
        self_pass_floor: u8,
        passability_cache: &PassabilityCache,
    ) -> Vec<ClientMessage> {
        if self.brains.is_empty() {
            return Vec::new();
        }
        // Self goes last, so an off-floor brain gets the same vec minus its
        // tail instead of a rebuilt one.
        let players: Vec<NearbyPlayer> = nearby_players
            .values()
            .chain(self_player)
            .map(|p| NearbyPlayer {
                id: p.id,
                position: p.position,
                health: p.health,
            })
            .collect();
        let without_self = players.len() - self_player.is_some() as usize;

        let path_provider = CachePathProvider {
            cache: passability_cache,
        };
        let mut rng = rand::thread_rng();

        let mut all_commands = Vec::new();
        let behavior_trees = &self.behavior_trees;
        for brain in self.brains.values_mut() {
            let Some(behavior_tree) =
                monster_ai::behavior_tree_for(behavior_trees, &brain.behavior)
            else {
                continue;
            };
            let visible = if brain.path_floor == self_pass_floor {
                &players[..]
            } else {
                &players[..without_self]
            };
            let result = brain.tick_with_behavior_tree(
                delta_ms,
                visible,
                behavior_tree,
                &path_provider,
                &mut rng,
            );
            all_commands.extend(result.commands.into_iter().map(command_to_client_msg));
        }
        all_commands
    }

    /// Check if we manage a given monster.
    pub fn manages(&self, monster_id: &str) -> bool {
        self.brains.contains_key(monster_id)
    }
}

fn command_to_client_msg(cmd: AiCommand) -> ClientMessage {
    match cmd {
        AiCommand::Move {
            monster_id,
            position,
            rotation,
            state,
            target_position,
        } => ClientMessage::MonsterMove {
            monster_id,
            position,
            rotation,
            state,
            target_position,
        },
        AiCommand::Attack {
            monster_id,
            target_player_id,
        } => ClientMessage::MonsterAttack {
            monster_id,
            target_player_id,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::{monster, test_player};

    fn npc_player(x: f32, z: f32) -> Player {
        Player {
            is_official_npc: true,
            ..test_player(x, z)
        }
    }

    fn manager_with_goblin(x: f32, z: f32) -> MonsterAiManager {
        let mut mgr = MonsterAiManager::new();
        mgr.set_behavior_trees(MonsterAiManager::load_behavior_trees_from_json(
            include_str!("../../data-src/behavior_trees.json"),
        ));
        mgr.add_monster(&Monster {
            monster_type: "goblin".to_string(),
            position: Position { x, y: 0.0, z },
            aggressive: true,
            ..monster("m1_1")
        });
        mgr
    }

    #[test]
    fn owned_monster_attacks_the_agents_own_npc() {
        let mut mgr = manager_with_goblin(1.0, 0.0);

        let me = npc_player(0.0, 0.0);
        let cache = PassabilityCache::new();
        let cmds = mgr.tick_all(100.0, &HashMap::new(), Some(&me), 0, &cache);

        assert!(
            cmds.iter().any(|c| matches!(
                c,
                ClientMessage::MonsterAttack { target_player_id, .. } if *target_player_id == me.id
            )),
            "monster ignored the NPC standing 1m away: {cmds:?}"
        );
    }

    #[test]
    fn owned_monster_ignores_its_npc_on_another_floor() {
        let mut mgr = manager_with_goblin(1.0, 0.0);

        let me = npc_player(0.0, 0.0);
        let cache = PassabilityCache::new();
        let dungeon_floor = onlinerpg_shared::dungeon::passability_floor_for_level(-1);
        let cmds = mgr.tick_all(100.0, &HashMap::new(), Some(&me), dungeon_floor, &cache);

        assert!(
            !cmds.iter().any(|c| matches!(
                c,
                ClientMessage::MonsterAttack { target_player_id, .. } if *target_player_id == me.id
            )),
            "monster attacked its NPC through the floor: {cmds:?}"
        );
    }
}

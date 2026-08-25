use super::*;
use crate::pathfinding::{PathResult, PathWaypoint};
use crate::world::{shortest_world_delta_x, WORLD_MAX_X, WORLD_MIN_X};
use crate::{MonsterState, PlayerId, Position};
use rand::rngs::SmallRng;
use rand::SeedableRng;

/// PathProvider that returns a straight-line path to the goal, over open ground.
struct DirectPath;
impl PathProvider for DirectPath {
    fn find_path(&self, _sx: f32, _sz: f32, _sf: u8, gx: f32, gz: f32, gf: u8) -> PathResult {
        PathResult {
            waypoints: vec![PathWaypoint {
                x: gx,
                z: gz,
                floor: gf,
            }],
            found: true,
        }
    }

    fn attack_line_blocked(&self, _fx: f32, _fz: f32, _tx: f32, _tz: f32, _floor: u8) -> bool {
        false
    }
}

/// Parameterized mock: straight-line paths to any goal `reach` allows, with
/// `blocked(fx, fz, tx, tz)` answering the attack-line query.
struct FnPath {
    reach: fn(f32, f32) -> bool,
    blocked: fn(f32, f32, f32, f32) -> bool,
}
impl PathProvider for FnPath {
    fn find_path(&self, _sx: f32, _sz: f32, _sf: u8, gx: f32, gz: f32, gf: u8) -> PathResult {
        if (self.reach)(gx, gz) {
            PathResult {
                waypoints: vec![PathWaypoint {
                    x: gx,
                    z: gz,
                    floor: gf,
                }],
                found: true,
            }
        } else {
            PathResult {
                waypoints: vec![],
                found: false,
            }
        }
    }

    fn attack_line_blocked(&self, fx: f32, fz: f32, tx: f32, tz: f32, _floor: u8) -> bool {
        (self.blocked)(fx, fz, tx, tz)
    }
}

/// A 1-wide corridor along x: only the z ∈ [10, 11) lane is walkable.
fn corridor_1_wide() -> FnPath {
    FnPath {
        reach: |_, gz| (10.0..11.0).contains(&gz),
        blocked: |_, _, _, _| false,
    }
}

/// A 2-wide corridor along x: two lanes, z ∈ [10, 12).
fn corridor_2_wide() -> FnPath {
    FnPath {
        reach: |_, gz| (10.0..12.0).contains(&gz),
        blocked: |_, _, _, _| false,
    }
}

/// Open ground, but every attack line is walled (a target up a stair).
fn walled_line() -> FnPath {
    FnPath {
        reach: |_, _| true,
        blocked: |_, _, _, _| true,
    }
}

/// No path anywhere and every line walled — a target that cannot be reached.
fn unreachable() -> FnPath {
    FnPath {
        reach: |_, _| false,
        blocked: |_, _, _, _| true,
    }
}

/// A door line at x=12: paths reach only goals at x >= 12, and any line
/// crossing the door is walled.
fn behind_door() -> FnPath {
    FnPath {
        reach: |gx, _| gx >= 12.0,
        blocked: |fx, _, tx, _| (fx < 12.0) != (tx < 12.0),
    }
}

fn make_brain() -> MonsterBrain {
    MonsterBrain::new(
        "test_m1".into(),
        // A type with no measured clip: brain tests set their own swing length
        // rather than inheriting one from the generated model data.
        "test_monster".into(),
        "default".into(),
        Position {
            x: 10.0,
            y: 0.0,
            z: 10.0,
        },
        10,
        10,
        1.0,
        8.0,
        DEFAULT_ATTACK_RANGE,
        DEFAULT_CHASE_RANGE,
        1500.0,
    )
}

#[test]
fn brain_starts_idle() {
    let brain = make_brain();
    assert_eq!(brain.state(), AiState::Idle);
    assert_eq!(brain.network_state(), MonsterState::Idle);
}

#[test]
fn idle_does_not_transition_before_check_interval() {
    let mut brain = make_brain();
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Action {
                    name: "wander".into(),
                    params: HashMap::from([("checkMs".into(), 1000.0)]),
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);

    let result = brain.tick_with_behavior_tree(500.0, &[], &[], &tree, &DirectPath, &mut rng);
    assert!(result.commands.is_empty());
    assert_eq!(brain.state(), AiState::Idle);
}

#[test]
fn idle_can_transition_to_move() {
    let mut brain = make_brain();
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Action {
            name: "wander".into(),
            params: HashMap::from([("checkMs".into(), 1000.0)]),
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);

    let result = brain.tick_with_behavior_tree(1001.0, &[], &[], &tree, &DirectPath, &mut rng);
    assert!(!result.commands.is_empty());
    assert!(brain.state() == AiState::Walk || brain.state() == AiState::Run);
}

#[test]
fn handle_hit_transitions_to_hit_state() {
    let mut brain = make_brain();

    let cmds = brain.handle_hit_with_behavior_tree(&1.into(), true, 3);
    assert!(!cmds.is_empty());
    assert_eq!(brain.state(), AiState::Hit);
    assert_eq!(brain.health, 7);
}

#[test]
fn handle_hit_death() {
    let mut brain = make_brain();

    let cmds = brain.handle_hit_with_behavior_tree(&1.into(), true, 100);
    assert!(cmds.is_empty()); // dead returns empty
    assert!(brain.is_dead());
    assert_eq!(brain.health, 0);
}

#[test]
fn load_behavior_trees_parses_json() {
    let trees = load_behavior_trees(include_str!("../../../data-src/behavior_trees.json"))
        .expect("behavior_trees.json should parse");

    assert!(trees.contains_key("timid"));
    assert!(trees.contains_key("brave"));
    assert!(behavior_tree_for(&trees, "missing").is_some());
}

#[test]
fn behavior_tree_attacks_target_in_range() {
    let mut brain = make_brain();
    brain.attack_cooldown_ms = 1000.0;
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 2.0)]),
                        },
                        BehaviorNode::Action {
                            name: "attack_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);

    let players = vec![NearbyPlayer {
        id: 1.into(),
        position: Position {
            x: 11.0,
            y: 0.0,
            z: 10.0,
        },
        health: 10,
    }];

    let result = brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);

    assert!(result
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));
    assert_eq!(brain.state(), AiState::Attack);
}

/// A wall between the two — a shut door, a stair wall. The server refuses such
/// a blow, so the brain must not throw it either.
#[test]
fn behavior_tree_holds_its_swing_through_a_wall() {
    struct WalledOff;
    impl PathProvider for WalledOff {
        fn find_path(
            &self,
            _sx: f32,
            _sz: f32,
            _sf: u8,
            _gx: f32,
            _gz: f32,
            _gf: u8,
        ) -> PathResult {
            PathResult {
                waypoints: Vec::new(),
                found: false,
            }
        }

        fn attack_line_blocked(&self, _fx: f32, _fz: f32, _tx: f32, _tz: f32, _floor: u8) -> bool {
            true
        }
    }

    let mut brain = make_brain();
    brain.attack_cooldown_ms = 1000.0;
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 2.0)]),
                        },
                        BehaviorNode::Action {
                            name: "attack_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);

    let players = vec![NearbyPlayer {
        id: 1.into(),
        position: Position {
            x: 11.0,
            y: 0.0,
            z: 10.0,
        },
        health: 10,
    }];

    let result = brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &WalledOff, &mut rng);

    assert!(!result
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));
    assert_ne!(brain.state(), AiState::Attack);
}

#[test]
fn chase_to_attack_fires_without_waiting_full_cooldown() {
    let mut brain = make_brain();
    brain.state = AiState::Chase;
    brain.target_player_id = Some(1.into());
    brain.attack_cooldown_ms = 4100.0;

    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "has_target".into(),
                            params: HashMap::new(),
                        },
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 2.0)]),
                        },
                        BehaviorNode::Action {
                            name: "attack_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);

    let players = vec![NearbyPlayer {
        id: 1.into(),
        position: Position {
            x: 11.9,
            y: 0.0,
            z: 10.0,
        },
        health: 10,
    }];

    let result = brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);

    assert!(result
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));
    assert_eq!(brain.state(), AiState::Attack);
}

#[test]
fn behavior_tree_chases_target_in_range() {
    let mut brain = make_brain();
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 25.0)]),
                        },
                        BehaviorNode::Action {
                            name: "chase_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);

    let players = vec![NearbyPlayer {
        id: 1.into(),
        position: Position {
            x: 15.0,
            y: 0.0,
            z: 10.0,
        },
        health: 10,
    }];

    let result = brain.tick_with_behavior_tree(50.0, &players, &[], &tree, &DirectPath, &mut rng);

    assert!(result
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Move { .. })));
    assert!(result.commands.iter().any(|c| {
        matches!(
            c,
            AiCommand::Move {
                state: MonsterState::Run,
                ..
            }
        )
    }));
    assert_eq!(brain.state(), AiState::Chase);
}

fn attacker_at(x: f32, z: f32) -> Vec<NearbyPlayer> {
    vec![NearbyPlayer {
        id: 1.into(),
        position: Position { x, y: 0.0, z },
        health: 10,
    }]
}

/// Bare attack action: a failed range check drops the brain to Idle without
/// moving it, so a test's positions stay exact.
fn attack_tree() -> BehaviorTree {
    BehaviorTree {
        description: None,
        root: BehaviorNode::Action {
            name: "attack_target".into(),
            params: HashMap::new(),
        },
    }
}

fn chase_tree() -> BehaviorTree {
    BehaviorTree {
        description: None,
        root: BehaviorNode::Sequence {
            children: vec![
                BehaviorNode::Condition {
                    name: "target_in_range".into(),
                    params: HashMap::from([("range".into(), 25.0)]),
                },
                BehaviorNode::Action {
                    name: "chase_target".into(),
                    params: HashMap::new(),
                },
            ],
        },
    }
}

fn leash_tree() -> BehaviorTree {
    BehaviorTree {
        description: None,
        root: BehaviorNode::Sequence {
            children: vec![
                BehaviorNode::Condition {
                    name: "is_beyond_leash".into(),
                    params: HashMap::from([("range".into(), 30.0)]),
                },
                BehaviorNode::Action {
                    name: "return_to_spawn".into(),
                    params: HashMap::new(),
                },
            ],
        },
    }
}

fn flee_tree() -> BehaviorTree {
    BehaviorTree {
        description: None,
        root: BehaviorNode::Sequence {
            children: vec![
                BehaviorNode::Condition {
                    name: "health_below_ratio".into(),
                    params: HashMap::from([("ratio".into(), 0.3)]),
                },
                BehaviorNode::Action {
                    name: "flee_from_target".into(),
                    params: HashMap::new(),
                },
            ],
        },
    }
}

#[test]
fn behavior_tree_flee_without_threat_position_runs_to_spawn() {
    let mut brain = make_brain();
    brain.position.x = 20.0;
    brain.target_player_id = Some(1.into());
    brain.health = 2;
    let tree = flee_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    let result = brain.tick_with_behavior_tree(16.0, &[], &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Flee);
    assert!(result.commands.iter().any(|c| {
        matches!(
            c,
            AiCommand::Move {
                state: MonsterState::Run,
                target_position: Position {
                    x: 10.0,
                    z: 10.0,
                    ..
                },
                ..
            }
        )
    }));
}

#[test]
fn behavior_tree_flee_runs_away_from_attacker_beyond_sight() {
    let mut brain = make_brain();
    brain.target_player_id = Some(1.into());
    brain.health = 2;
    let tree = flee_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    // Attacker just west of the monster — flee leg must point east, one
    // full safe distance (chase 25 + margin 5) away.
    let players = attacker_at(8.0, 10.0);

    let result = brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Flee);
    assert!(result.commands.iter().any(|c| {
        matches!(
            c,
            AiCommand::Move {
                state: MonsterState::Run,
                target_position: Position { x, z, .. },
                ..
            } if (x - 40.0).abs() < 0.01 && (z - 10.0).abs() < 0.01
        )
    }));
}

#[test]
fn behavior_tree_flee_stops_once_beyond_safe_distance() {
    let mut brain = make_brain();
    brain.target_player_id = Some(1.into());
    brain.health = 2;
    let tree = flee_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    let players = attacker_at(8.0, 10.0);

    brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert_eq!(brain.state(), AiState::Flee);

    // Large delta covers the whole flee leg: monster ends at x=40,
    // 32m from the attacker — beyond the 30m safe distance.
    brain.tick_with_behavior_tree(5000.0, &players, &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Idle);
    assert!(brain.target_player_id.is_none());
    assert!((brain.position.x - 40.0).abs() < 0.01);
}

#[test]
fn behavior_tree_flee_repaths_when_attacker_keeps_chasing() {
    let mut brain = make_brain();
    brain.target_player_id = Some(1.into());
    brain.health = 2;
    let tree = flee_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    let players = attacker_at(8.0, 10.0);
    brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert_eq!(brain.state(), AiState::Flee);

    // Attacker chased to x=35 — when the first leg ends at x=40 the
    // monster is still within sight, so it must start another leg east.
    let chasing = attacker_at(35.0, 10.0);
    let result = brain.tick_with_behavior_tree(5000.0, &chasing, &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Flee);
    assert!(result.commands.iter().any(|c| {
        matches!(
            c,
            AiCommand::Move {
                target_position: Position { x, .. },
                ..
            } if (x - 70.0).abs() < 0.01
        )
    }));
}

#[test]
fn behavior_tree_does_not_flee_without_target() {
    let mut brain = make_brain();
    brain.position.x = 20.0;
    brain.health = 2;
    let tree = flee_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    let result = brain.tick_with_behavior_tree(16.0, &[], &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Idle);
    assert!(result.commands.is_empty());
}

#[test]
fn behavior_tree_return_sends_walk_target_to_spawn() {
    let mut brain = make_brain();
    brain.position.x = 70.0;
    let tree = leash_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    let result = brain.tick_with_behavior_tree(16.0, &[], &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Return);
    assert!(result.commands.iter().any(|c| {
        matches!(
            c,
            AiCommand::Move {
                state: MonsterState::Walk,
                target_position: Position {
                    x: 10.0,
                    z: 10.0,
                    ..
                },
                ..
            }
        )
    }));
}

#[test]
fn behavior_tree_requires_existing_target_before_attacking() {
    let mut brain = make_brain();
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "has_target".into(),
                            params: HashMap::new(),
                        },
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 2.0)]),
                        },
                        BehaviorNode::Action {
                            name: "attack_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);
    let players = vec![NearbyPlayer {
        id: 1.into(),
        position: Position {
            x: 11.0,
            y: 0.0,
            z: 10.0,
        },
        health: 10,
    }];

    let peaceful = brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert!(!peaceful
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));
    assert_eq!(brain.state(), AiState::Idle);

    brain.handle_hit_with_behavior_tree(&1.into(), false, 0);
    let provoked = brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert!(provoked
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));
}

#[test]
fn provocation_interrupts_an_in_progress_wander() {
    let mut brain = make_brain();
    let mut commands = Vec::new();
    let mut rng = SmallRng::seed_from_u64(42);

    brain.transition_to_move(&mut commands, 10.0, 11.0, &DirectPath, &mut rng);
    assert!(matches!(brain.state(), AiState::Walk | AiState::Run));

    let provoke_commands = brain.handle_hit_with_behavior_tree(&1.into(), false, 0);

    assert_eq!(brain.state(), AiState::Idle);
    assert_eq!(brain.target_player_id, Some(PlayerId::from(1)));
    assert!(brain.target_position.is_none());
    assert!(brain.waypoints.is_empty());
    assert!(provoke_commands.iter().any(|command| matches!(
        command,
        AiCommand::Move {
            state: MonsterState::Idle,
            ..
        }
    )));
}

#[test]
fn attack_chases_nearby_player() {
    let mut brain = make_brain();
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "has_target".into(),
                            params: HashMap::new(),
                        },
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 25.0)]),
                        },
                        BehaviorNode::Action {
                            name: "chase_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);

    brain.state = AiState::Attack;
    brain.target_player_id = Some(1.into());
    brain.move_speed = brain.run_speed;

    let players = vec![NearbyPlayer {
        id: 1.into(),
        position: Position {
            x: 15.0,
            y: 0.0,
            z: 10.0,
        },
        health: 10,
    }];

    let result = brain.tick_with_behavior_tree(50.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert!(result
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Move { .. })));
}

#[test]
fn attack_command_uses_monster_cooldown() {
    let mut brain = make_brain();
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "has_target".into(),
                            params: HashMap::new(),
                        },
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 2.0)]),
                        },
                        BehaviorNode::Action {
                            name: "attack_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);

    brain.state = AiState::Attack;
    brain.target_player_id = Some(1.into());
    brain.attack_cooldown_ms = 1800.0;
    // Mid-combat: it has just swung, so the next one waits out this monster's
    // own cooldown rather than any default.
    brain.attack_cooldown_left_ms = 1800.0;

    let players = attacker_at(11.0, 10.0);

    let before_cooldown =
        brain.tick_with_behavior_tree(1700.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert!(!before_cooldown
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));

    let after_cooldown =
        brain.tick_with_behavior_tree(100.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert!(after_cooldown
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));
}

/// Leaving attack range and coming back must not re-arm the swing: the cooldown
/// used to live in the state timer that entering Attack reset.
#[test]
fn re_entering_attack_range_does_not_re_arm_the_cooldown() {
    let mut brain = make_brain();
    brain.target_player_id = Some(1.into());
    // Attack alone: a failed range check drops the brain to Idle without moving
    // it, so the positions below stay exact across the flap.
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Sequence {
            children: vec![
                BehaviorNode::Condition {
                    name: "target_in_range".into(),
                    params: HashMap::from([("range".into(), 2.0)]),
                },
                BehaviorNode::Action {
                    name: "attack_target".into(),
                    params: HashMap::new(),
                },
            ],
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);
    let near = attacker_at(11.0, 10.0);
    let far = attacker_at(13.0, 10.0);

    let mut attacks = 0;
    let mut entries = 0;
    for tick in 0..20 {
        let players = if tick % 2 == 0 { &near } else { &far };
        let was_attacking = brain.state() == AiState::Attack;
        let result =
            brain.tick_with_behavior_tree(16.0, players, &[], &tree, &DirectPath, &mut rng);
        if !was_attacking && brain.state() == AiState::Attack {
            entries += 1;
        }
        attacks += result
            .commands
            .iter()
            .filter(|c| matches!(c, AiCommand::Attack { .. }))
            .count();
    }

    assert!(
        entries > 1,
        "the flap should re-enter Attack, got {entries}"
    );
    assert_eq!(
        attacks, 1,
        "320ms of flapping is one cooldown, so one swing — not one per entry"
    );
}

/// The attack holds out to a wider radius than it engages at, but must not
/// engage from out there. See ATTACK_RELEASE_MARGIN_METERS.
#[test]
fn attack_holds_past_its_range_but_engages_only_inside_it() {
    let tree = attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    // 2.3m from the monster at x=10: outside the 2m engage radius, inside the
    // 2.5m release radius.
    let players = attacker_at(12.3, 10.0);

    let mut holding = make_brain();
    holding.attack_range = 2.0;
    holding.target_player_id = Some(1.into());
    holding.state = AiState::Attack;
    holding.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert_eq!(
        holding.state(),
        AiState::Attack,
        "an attack must not drop the frame the target steps outside it"
    );

    let mut engaging = make_brain();
    engaging.attack_range = 2.0;
    engaging.target_player_id = Some(1.into());
    engaging.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);
    assert_ne!(
        engaging.state(),
        AiState::Attack,
        "the wider release radius must not let it engage from out of range"
    );
}

/// A swing already under way finishes even if the target walks off.
#[test]
fn a_swing_in_progress_is_not_abandoned_when_the_target_walks_off() {
    let tree = attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let mut brain = make_brain();
    brain.attack_range = 2.0;
    brain.swing_commit_ms = 450.0;
    brain.target_player_id = Some(1.into());

    let near = attacker_at(11.0, 10.0);
    let result = brain.tick_with_behavior_tree(16.0, &near, &[], &tree, &DirectPath, &mut rng);
    assert!(result
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));

    // The target sprints clear, but the swing is only one frame old.
    let far = attacker_at(20.0, 10.0);
    brain.tick_with_behavior_tree(16.0, &far, &[], &tree, &DirectPath, &mut rng);
    assert_eq!(
        brain.state(),
        AiState::Attack,
        "the swing must land before the monster gives it up"
    );

    brain.tick_with_behavior_tree(450.0, &far, &[], &tree, &DirectPath, &mut rng);
    assert_ne!(
        brain.state(),
        AiState::Attack,
        "once it has landed the attack releases"
    );
}

/// L-shaped path: out along X, then along Z, so a leg has one interior corner.
struct BentPath;
impl PathProvider for BentPath {
    fn find_path(&self, _sx: f32, sz: f32, sf: u8, gx: f32, gz: f32, gf: u8) -> PathResult {
        PathResult {
            waypoints: vec![
                PathWaypoint {
                    x: gx,
                    z: sz,
                    floor: sf,
                },
                PathWaypoint {
                    x: gx,
                    z: gz,
                    floor: gf,
                },
            ],
            found: true,
        }
    }

    fn attack_line_blocked(&self, _fx: f32, _fz: f32, _tx: f32, _tz: f32, _floor: u8) -> bool {
        false
    }
}

/// The server only sees the straight line between two reported positions, and
/// refuses it when it crosses solid ground. So a corner must land on a report
/// rather than inside one, or rounding it looks like walking through the wall.
#[test]
fn a_reported_move_never_spans_a_path_bend() {
    let mut brain = make_brain();
    brain.target_player_id = Some(1.into());
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Action {
            name: "chase_target".into(),
            // Hold the one path so the only bend is its corner.
            params: HashMap::from([("pathRecalcMs".into(), 1.0e9)]),
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(30.0, 40.0);

    let mut reported = Vec::new();
    for _ in 0..400 {
        let result = brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &BentPath, &mut rng);
        for cmd in &result.commands {
            if let AiCommand::Move { position, .. } = cmd {
                reported.push(*position);
            }
        }
        // Stop once past the corner; the rest of the leg proves nothing.
        if brain.position.z > 10.0 {
            break;
        }
    }

    // The corner sits at the chase goal's x (a free cell near the target, so
    // not exactly the target's x) — after it the brain moves only in z.
    let corner = Position {
        x: brain.position.x,
        y: 0.0,
        z: 10.0,
    };
    assert!(
        reported
            .iter()
            .any(|p| (p.x - corner.x).abs() < 0.01 && (p.z - corner.z).abs() < 0.01),
        "the corner must be reported, got {reported:?}"
    );
}

/// The server refuses a move by echoing back the position it kept. The brain has
/// to resume from there — carrying on from its own would repeat the refusal
/// forever, leaving the monster frozen for everyone but its owner.
#[test]
fn a_correction_moves_the_brain_back_and_repaths() {
    let mut brain = make_brain();
    brain.target_player_id = Some(1.into());
    let tree = BehaviorTree {
        description: None,
        root: BehaviorNode::Action {
            name: "chase_target".into(),
            params: HashMap::new(),
        },
    };
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(40.0, 10.0);

    // BentPath's corner sits at the mover's own z, so a rebuilt path is
    // distinguishable from the stale one the correction has to discard.
    for _ in 0..20 {
        brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &BentPath, &mut rng);
    }
    assert!(
        brain.position.x > 11.0,
        "the chase should have advanced first, got {}",
        brain.position.x
    );

    let kept = Position {
        x: 10.5,
        y: 0.0,
        z: 25.0,
    };
    brain.apply_authoritative_position(kept);
    assert_eq!(brain.position, kept);

    brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &BentPath, &mut rng);

    assert!(
        brain.position.x < 11.0,
        "the next step must start from the corrected position, got {}",
        brain.position.x
    );
    assert_eq!(
        brain.waypoints.first().map(|w| w.z),
        Some(25.0),
        "the path must be rebuilt from the corrected position, not resumed"
    );
}

/// Same for a fleeing monster: its path is rebuilt from the threat's direction
/// rather than followed from a position the server threw away.
#[test]
fn a_correction_repaths_a_fleeing_monster() {
    let mut brain = make_brain();
    brain.position.x = 20.0;
    brain.health = 2;
    brain.target_player_id = Some(1.into());
    let tree = flee_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(18.0, 10.0);

    for _ in 0..20 {
        brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);
    }
    assert_eq!(brain.state(), AiState::Flee);

    let kept = Position {
        x: 20.0,
        y: 0.0,
        z: 10.0,
    };
    brain.apply_authoritative_position(kept);
    brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);

    assert_eq!(
        brain.state(),
        AiState::Flee,
        "a correction must not end the flee"
    );
    assert!(!brain.waypoints.is_empty(), "the brain must have repathed");
}

/// The world is a cylinder in X. A monster at the east edge chasing a player
/// just across the seam has to step east into the wrap, not crawl a whole
/// world width west — and its stored position stays canonical, the same
/// convention the server's movement sweep keeps.
#[test]
fn chase_across_world_seam_takes_the_short_way() {
    let mut brain = make_brain();
    let start = Position {
        x: WORLD_MAX_X - 0.2,
        y: 0.0,
        z: 10.0,
    };
    brain.position = start;
    brain.spawn_position = start;
    let tree = chase_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    // 3.2m east of the monster, on the far side of the seam.
    let players = attacker_at(WORLD_MIN_X + 3.0, 10.0);

    brain.tick_with_behavior_tree(50.0, &players, &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Chase);
    let moved = shortest_world_delta_x(start.x, brain.position.x);
    assert!(
        moved > 0.0 && moved < 1.0,
        "expected a short step east across the seam, got {moved}"
    );
    assert!(
        brain.position.x >= WORLD_MIN_X && brain.position.x < WORLD_MAX_X,
        "stored X must stay canonical, got {}",
        brain.position.x
    );
}

/// The flee leg points away from the threat, so its delta runs threat -> self.
/// Across the seam a raw subtraction flips that sign and runs the monster into
/// its attacker.
#[test]
fn flee_across_world_seam_runs_away_from_the_threat() {
    let mut brain = make_brain();
    let start = Position {
        x: WORLD_MIN_X + 1.0,
        y: 0.0,
        z: 10.0,
    };
    brain.position = start;
    brain.spawn_position = start;
    brain.target_player_id = Some(1.into());
    brain.health = 2;
    let tree = flee_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    // Attacker 2m west, on the far side of the seam.
    let players = attacker_at(WORLD_MAX_X - 1.0, 10.0);

    brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Flee);
    let destination = brain.target_position.expect("flee picks a destination");
    let away = shortest_world_delta_x(start.x, destination.x);
    assert!(
        away > 0.0,
        "the flee leg must point east, away from the attacker, got {away}"
    );
}

/// Leash range is a distance, so it takes the periodic one: a monster two
/// meters from its spawn across the seam has not wandered a world width.
#[test]
fn leash_measures_periodic_distance_to_spawn() {
    let mut brain = make_brain();
    brain.spawn_position = Position {
        x: WORLD_MAX_X - 1.0,
        y: 0.0,
        z: 10.0,
    };
    brain.position = Position {
        x: WORLD_MIN_X + 1.0,
        y: 0.0,
        z: 10.0,
    };
    let tree = leash_tree();
    let mut rng = SmallRng::seed_from_u64(42);

    let result = brain.tick_with_behavior_tree(16.0, &[], &[], &tree, &DirectPath, &mut rng);

    assert_ne!(brain.state(), AiState::Return);
    // The leash condition has to fail outright. Letting it pass and relying on
    // `return_to_spawn` to notice it already arrived would still emit a pose.
    assert!(
        result.commands.is_empty(),
        "a monster inside its leash must not report a return"
    );
}

// =========================================================================
// Cell separation (doc/MONSTER_SEPARATION.md)
// =========================================================================

fn chase_attack_tree() -> BehaviorTree {
    BehaviorTree {
        description: None,
        root: BehaviorNode::Selector {
            children: vec![
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 25.0)]),
                        },
                        BehaviorNode::Action {
                            name: "attack_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Sequence {
                    children: vec![
                        BehaviorNode::Condition {
                            name: "target_in_range".into(),
                            params: HashMap::from([("range".into(), 25.0)]),
                        },
                        BehaviorNode::Action {
                            name: "chase_target".into(),
                            params: HashMap::new(),
                        },
                    ],
                },
                BehaviorNode::Action {
                    name: "idle".into(),
                    params: HashMap::new(),
                },
            ],
        },
    }
}

fn brain_at(id: &str, x: f32, z: f32) -> MonsterBrain {
    MonsterBrain::new(
        id.into(),
        "test_monster".into(),
        "default".into(),
        Position { x, y: 0.0, z },
        10,
        10,
        1.0,
        8.0,
        DEFAULT_ATTACK_RANGE,
        DEFAULT_CHASE_RANGE,
        1500.0,
    )
}

fn assert_distinct_cells(brains: &[&MonsterBrain], msg: &str) {
    let cells: Vec<_> = brains
        .iter()
        .map(|b| cell_of(b.position.x, b.position.z))
        .collect();
    for i in 0..cells.len() {
        for j in i + 1..cells.len() {
            assert!(cells[i] != cells[j], "{msg}: {cells:?}");
        }
    }
}

fn view_of(brains: &[&MonsterBrain]) -> Vec<NearbyMonster> {
    brains
        .iter()
        .map(|b| NearbyMonster {
            id: b.monster_id.clone(),
            position: b.position,
            state: b.network_state(),
            path_floor: b.path_floor,
        })
        .collect()
}

#[test]
fn chase_goal_avoids_a_cell_occupied_by_a_stander() {
    let mut brain = brain_at("m1", 14.0, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.0, 10.5);
    let stander = vec![NearbyMonster {
        id: "m2".into(),
        position: Position {
            x: 11.5,
            y: 0.0,
            z: 10.5,
        },
        state: MonsterState::Idle,
        path_floor: 0,
    }];

    for _ in 0..400 {
        brain.tick_with_behavior_tree(16.0, &players, &stander, &tree, &DirectPath, &mut rng);
        if brain.state() == AiState::Attack {
            break;
        }
    }

    assert_eq!(brain.state(), AiState::Attack);
    assert_ne!(
        cell_of(brain.position.x, brain.position.z),
        (11, 10),
        "must stand beside the occupied cell, not in it"
    );
}

#[test]
fn a_corridor_queues_chasers_one_per_cell() {
    let mut b1 = brain_at("m1", 13.0, 10.5);
    let mut b2 = brain_at("m2", 15.0, 10.5);
    let mut b3 = brain_at("m3", 17.0, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.0, 10.5);

    let mut b2_last_move_step = 0;
    let mut b2_hold_cmd = false;
    for step in 0..600 {
        let v1 = view_of(&[&b2, &b3]);
        b1.tick_with_behavior_tree(16.0, &players, &v1, &tree, &corridor_1_wide(), &mut rng);
        let v2 = view_of(&[&b1, &b3]);
        let r2 =
            b2.tick_with_behavior_tree(16.0, &players, &v2, &tree, &corridor_1_wide(), &mut rng);
        for cmd in &r2.commands {
            if let AiCommand::Move {
                state,
                position,
                target_position,
                ..
            } = cmd
            {
                b2_last_move_step = step;
                if *state == MonsterState::Idle && *position == *target_position {
                    b2_hold_cmd = true;
                }
            }
        }
        let v3 = view_of(&[&b1, &b2]);
        b3.tick_with_behavior_tree(16.0, &players, &v3, &tree, &corridor_1_wide(), &mut rng);
    }

    assert_eq!(b1.state(), AiState::Attack);
    assert_eq!(b2.network_state(), MonsterState::Idle, "b2 queues");
    assert_eq!(b3.network_state(), MonsterState::Idle, "b3 queues");

    assert_distinct_cells(&[&b1, &b2, &b3], "one per cell");
    assert!(b1.position.x < b2.position.x && b2.position.x < b3.position.x);

    // The hold reported one idle pose, then went quiet.
    assert!(b2_hold_cmd, "hold must sync an idle pose");
    assert!(
        b2_last_move_step < 500,
        "a settled queue stops emitting moves, last at {b2_last_move_step}"
    );

    // The head dies — the queue advances.
    b1.handle_death();
    for _ in 0..600 {
        let v2 = view_of(&[&b1, &b3]);
        b2.tick_with_behavior_tree(16.0, &players, &v2, &tree, &corridor_1_wide(), &mut rng);
        let v3 = view_of(&[&b1, &b2]);
        b3.tick_with_behavior_tree(16.0, &players, &v3, &tree, &corridor_1_wide(), &mut rng);
    }
    assert_eq!(b2.state(), AiState::Attack, "second in line advances");
}

#[test]
fn no_free_cell_falls_back_to_the_raw_target_position() {
    // Reach so short (0.8m) that no other cell's center is in range of a
    // target standing dead-center in its own cell: no valid standing cell.
    let mut brain = MonsterBrain::new(
        "m1".into(),
        "test_monster".into(),
        "default".into(),
        Position {
            x: 14.0,
            y: 0.0,
            z: 10.5,
        },
        10,
        10,
        1.0,
        8.0,
        0.8,
        DEFAULT_CHASE_RANGE,
        1500.0,
    );
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.5, 10.5);

    for _ in 0..400 {
        brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &DirectPath, &mut rng);
        if brain.state() == AiState::Attack {
            break;
        }
    }
    assert_eq!(brain.state(), AiState::Attack);
}

/// A 2-wide corridor seats two attackers: the second chaser sidesteps into
/// the other lane instead of queueing behind the first.
#[test]
fn a_two_wide_corridor_seats_two_attackers() {
    let mut b1 = brain_at("m1", 13.0, 10.5);
    let mut b2 = brain_at("m2", 15.0, 10.5);
    let mut b3 = brain_at("m3", 17.0, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.0, 10.5);

    for _ in 0..600 {
        let v1 = view_of(&[&b2, &b3]);
        b1.tick_with_behavior_tree(16.0, &players, &v1, &tree, &corridor_2_wide(), &mut rng);
        let v2 = view_of(&[&b1, &b3]);
        b2.tick_with_behavior_tree(16.0, &players, &v2, &tree, &corridor_2_wide(), &mut rng);
        let v3 = view_of(&[&b1, &b2]);
        b3.tick_with_behavior_tree(16.0, &players, &v3, &tree, &corridor_2_wide(), &mut rng);
    }

    // Which chaser wins the second lane is a race — assert the shape, not
    // the identities: two attackers in different lanes, one queued.
    let brains = [&b1, &b2, &b3];
    let attackers: Vec<_> = brains
        .iter()
        .filter(|b| b.state() == AiState::Attack)
        .collect();
    let holders: Vec<_> = brains
        .iter()
        .filter(|b| b.network_state() == MonsterState::Idle)
        .collect();
    assert_eq!(attackers.len(), 2, "a 2-wide corridor seats two attackers");
    assert_eq!(holders.len(), 1, "the third queues");
    assert_ne!(
        cell_of(attackers[0].position.x, attackers[0].position.z).1,
        cell_of(attackers[1].position.x, attackers[1].position.z).1,
        "the two attackers sit in different lanes"
    );

    assert_distinct_cells(&[&b1, &b2, &b3], "one per cell");
}

#[test]
fn a_walled_target_is_chased_to_its_own_position_not_a_nearby_cell() {
    let mut brain = brain_at("m1", 14.0, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.0, 10.5);

    for _ in 0..400 {
        brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &walled_line(), &mut rng);
    }

    // Never attacks through the wall, and never parks on a ring cell it
    // cannot attack from — it heads for the target position itself, which is
    // what carries a stair climb in the real pathfinder.
    assert_ne!(brain.state(), AiState::Attack);
    let dx = brain.position.x - 10.0;
    let dz = brain.position.z - 10.5;
    assert!(
        (dx * dx + dz * dz).sqrt() < 0.5,
        "must chase the raw target position, got ({:.2},{:.2})",
        brain.position.x,
        brain.position.z
    );
}

#[test]
fn unreachable_target_holds_in_place_instead_of_wandering() {
    let mut brain = brain_at("m1", 14.0, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.0, 10.5);

    let mut cmds = 0;
    for _ in 0..300 {
        let r = brain.tick_with_behavior_tree(16.0, &players, &[], &tree, &unreachable(), &mut rng);
        cmds += r.commands.len();
    }

    // Waiting (not failed into idle/wander), shown as idle, unmoved,
    // and quiet after the one hold pose.
    assert_eq!(brain.state(), AiState::Hold);
    assert_eq!(brain.network_state(), MonsterState::Idle);
    assert_eq!((brain.position.x, brain.position.z), (14.0, 10.5));
    assert!(cmds <= 2, "a quiet hold, got {cmds} commands");
}

#[test]
fn stacked_attackers_spread_to_one_per_cell() {
    let mut b1 = brain_at("m1", 14.0, 10.5);
    let mut b2 = brain_at("m2", 14.0, 10.5);
    let mut b3 = brain_at("m3", 14.0, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.0, 10.5);

    for _ in 0..600 {
        let v1 = view_of(&[&b2, &b3]);
        b1.tick_with_behavior_tree(16.0, &players, &v1, &tree, &DirectPath, &mut rng);
        let v2 = view_of(&[&b1, &b3]);
        b2.tick_with_behavior_tree(16.0, &players, &v2, &tree, &DirectPath, &mut rng);
        let v3 = view_of(&[&b1, &b2]);
        b3.tick_with_behavior_tree(16.0, &players, &v3, &tree, &DirectPath, &mut rng);
    }

    assert_eq!(b1.state(), AiState::Attack);
    assert_eq!(b2.state(), AiState::Attack);
    assert_eq!(b3.state(), AiState::Attack);
    assert_distinct_cells(&[&b1, &b2, &b3], "stacked arrivals must spread");
}

#[test]
fn stacked_door_waiters_spread_one_per_cell() {
    let mut b1 = brain_at("m1", 13.5, 10.5);
    let mut b2 = brain_at("m2", 13.5, 10.5);
    let mut b3 = brain_at("m3", 13.5, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.0, 10.5);

    for _ in 0..600 {
        let v1 = view_of(&[&b2, &b3]);
        b1.tick_with_behavior_tree(16.0, &players, &v1, &tree, &behind_door(), &mut rng);
        let v2 = view_of(&[&b1, &b3]);
        b2.tick_with_behavior_tree(16.0, &players, &v2, &tree, &behind_door(), &mut rng);
        let v3 = view_of(&[&b1, &b2]);
        b3.tick_with_behavior_tree(16.0, &players, &v3, &tree, &behind_door(), &mut rng);
    }

    for b in [&b1, &b2, &b3] {
        assert_eq!(b.state(), AiState::Hold);
        assert_eq!(b.network_state(), MonsterState::Idle, "waits at the door");
    }
    assert_distinct_cells(&[&b1, &b2, &b3], "door waiters must spread");
}

/// The door shuts while the pack is mid-chase: everyone must settle into a
/// quiet hold (spread one per cell), not keep milling about.
#[test]
fn door_closing_mid_chase_settles_the_pack() {
    use std::cell::Cell;
    struct Door {
        open: Cell<bool>,
    }
    impl PathProvider for Door {
        fn find_path(&self, _sx: f32, _sz: f32, _sf: u8, gx: f32, gz: f32, gf: u8) -> PathResult {
            let lanes = (10.0..12.0).contains(&gz);
            if lanes && (self.open.get() || gx >= 12.0) {
                PathResult {
                    waypoints: vec![PathWaypoint {
                        x: gx,
                        z: gz,
                        floor: gf,
                    }],
                    found: true,
                }
            } else if lanes {
                // Like the real A*: an unreachable goal answers with a
                // partial path to the closest reachable cell (found=false).
                PathResult {
                    waypoints: vec![PathWaypoint {
                        x: 12.2,
                        z: gz,
                        floor: gf,
                    }],
                    found: false,
                }
            } else {
                PathResult {
                    waypoints: vec![],
                    found: false,
                }
            }
        }

        fn attack_line_blocked(&self, fx: f32, _fz: f32, tx: f32, _tz: f32, _floor: u8) -> bool {
            !self.open.get() && (fx < 12.0) != (tx < 12.0)
        }
    }

    let door = Door {
        open: Cell::new(true),
    };
    let mut b1 = brain_at("m1", 18.0, 10.5);
    let mut b2 = brain_at("m2", 19.5, 10.5);
    let mut b3 = brain_at("m3", 21.0, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(10.0, 10.5);

    // The server refuses a move through the shut door by echoing back the
    // kept position; the owner snaps the brain there.
    fn enforce_door(brain: &mut MonsterBrain, before: Position, open: bool) {
        if !open && before.x >= 12.0 && brain.position.x < 12.0 {
            brain.apply_authoritative_position(before);
        }
    }

    let mut moved_late = 0.0_f32;
    for step in 0..900 {
        if step == 20 {
            door.open.set(false);
        }
        let before = [b1.position, b2.position, b3.position];
        let v1 = view_of(&[&b2, &b3]);
        b1.tick_with_behavior_tree(16.0, &players, &v1, &tree, &door, &mut rng);
        enforce_door(&mut b1, before[0], door.open.get());
        let v2 = view_of(&[&b1, &b3]);
        b2.tick_with_behavior_tree(16.0, &players, &v2, &tree, &door, &mut rng);
        enforce_door(&mut b2, before[1], door.open.get());
        let v3 = view_of(&[&b1, &b2]);
        b3.tick_with_behavior_tree(16.0, &players, &v3, &tree, &door, &mut rng);
        enforce_door(&mut b3, before[2], door.open.get());
        if step >= 700 {
            for (b, p) in [&b1, &b2, &b3].iter().zip(before.iter()) {
                let dx = b.position.x - p.x;
                let dz = b.position.z - p.z;
                moved_late += (dx * dx + dz * dz).sqrt();
            }
        }
    }

    assert!(
        moved_late < 0.01,
        "the pack must settle, moved {moved_late}m in the last 200 steps"
    );
    for b in [&b1, &b2, &b3] {
        assert_eq!(b.network_state(), MonsterState::Idle, "quiet wait");
    }
    assert_distinct_cells(&[&b1, &b2, &b3], "door pack spreads");
}

/// Every path detours through cell (10, 11) first — the shape a wall gives
/// A* — and a stander sits in that cell. The sidestep must reject legs that
/// cross an occupied cell, or it re-picks the same blocked sidestep every
/// tick and jogs in place forever.
#[test]
fn sidestep_rejects_a_leg_through_an_occupied_cell() {
    struct NorthDetour;
    impl PathProvider for NorthDetour {
        fn find_path(&self, _sx: f32, _sz: f32, _sf: u8, gx: f32, gz: f32, gf: u8) -> PathResult {
            PathResult {
                waypoints: vec![
                    PathWaypoint {
                        x: 10.5,
                        z: 11.5,
                        floor: gf,
                    },
                    PathWaypoint {
                        x: gx,
                        z: gz,
                        floor: gf,
                    },
                ],
                found: true,
            }
        }

        fn attack_line_blocked(&self, _fx: f32, _fz: f32, _tx: f32, _tz: f32, _floor: u8) -> bool {
            false
        }
    }

    let mut brain = brain_at("m1", 10.5, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(12.5, 9.0);
    let stander = vec![NearbyMonster {
        id: "m2".into(),
        position: Position {
            x: 10.5,
            y: 0.0,
            z: 11.5,
        },
        state: MonsterState::Idle,
        path_floor: 0,
    }];

    let mut moved_late = 0.0_f32;
    for step in 0..300 {
        let before = brain.position;
        brain.tick_with_behavior_tree(16.0, &players, &stander, &tree, &NorthDetour, &mut rng);
        if step >= 150 {
            let dx = brain.position.x - before.x;
            let dz = brain.position.z - before.z;
            moved_late += (dx * dx + dz * dz).sqrt();
        }
    }

    assert!(
        moved_late < 0.01,
        "must settle instead of jogging in place, moved {moved_late}m late"
    );
    assert_eq!(brain.network_state(), MonsterState::Idle, "quiet hold");
    assert!(brain.position.z < 11.0, "never entered the occupied cell");
}

/// The route to the goal dips south around a wall, but a stander blocks the
/// dip. The north pocket is euclidean-closer to the goal yet route-farther —
/// a sidestep there would be undone by the next repath, oscillating forever.
/// The route-length check must reject it so the chaser queues quietly.
#[test]
fn sidestep_rejects_a_euclidean_shortcut_that_is_route_farther() {
    struct DipMaze;
    impl PathProvider for DipMaze {
        fn find_path(&self, sx: f32, _sz: f32, _sf: u8, gx: f32, gz: f32, gf: u8) -> PathResult {
            // Goals in the slot zone (east-north) are reachable only via a
            // southern dip; everything else is a straight line.
            if gz > 11.0 && gx > 11.0 {
                PathResult {
                    waypoints: vec![
                        PathWaypoint {
                            x: sx,
                            z: 9.5,
                            floor: gf,
                        },
                        PathWaypoint {
                            x: gx,
                            z: gz,
                            floor: gf,
                        },
                    ],
                    found: true,
                }
            } else {
                PathResult {
                    waypoints: vec![PathWaypoint {
                        x: gx,
                        z: gz,
                        floor: gf,
                    }],
                    found: true,
                }
            }
        }

        fn attack_line_blocked(&self, _fx: f32, _fz: f32, _tx: f32, _tz: f32, _floor: u8) -> bool {
            false
        }
    }

    let mut brain = brain_at("m1", 10.5, 10.5);
    let tree = chase_attack_tree();
    let mut rng = SmallRng::seed_from_u64(42);
    let players = attacker_at(12.5, 11.9);
    let stander = |x: f32, z: f32, id: &str| NearbyMonster {
        id: id.into(),
        position: Position { x, y: 0.0, z },
        state: MonsterState::Idle,
        path_floor: 0,
    };
    // South dip blocked, east/west taken: the only free neighbor is the
    // north pocket.
    let others = vec![
        stander(10.5, 9.5, "m2"),
        stander(11.5, 10.5, "m3"),
        stander(9.5, 10.5, "m4"),
    ];

    let mut moved_late = 0.0_f32;
    for step in 0..300 {
        let before = brain.position;
        brain.tick_with_behavior_tree(16.0, &players, &others, &tree, &DipMaze, &mut rng);
        if step >= 150 {
            let dx = brain.position.x - before.x;
            let dz = brain.position.z - before.z;
            moved_late += (dx * dx + dz * dz).sqrt();
        }
    }

    assert!(
        moved_late < 0.01,
        "must queue instead of oscillating, moved {moved_late}m late"
    );
    assert_eq!(brain.network_state(), MonsterState::Idle, "quiet hold");
    assert!(brain.position.z < 11.0, "never wandered into the pocket");
}

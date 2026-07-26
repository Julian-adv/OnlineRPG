use super::*;
use crate::pathfinding::{PathResult, PathWaypoint};
use crate::{MonsterState, PlayerId, Position};
use rand::rngs::SmallRng;
use rand::SeedableRng;

/// PathProvider that returns a straight-line path to the goal.
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

    let result = brain.tick_with_behavior_tree(500.0, &[], &tree, &DirectPath, &mut rng);
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

    let result = brain.tick_with_behavior_tree(1001.0, &[], &tree, &DirectPath, &mut rng);
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

    let result = brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);

    assert!(result
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));
    assert_eq!(brain.state(), AiState::Attack);
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

    let result = brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);

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

    let result = brain.tick_with_behavior_tree(50.0, &players, &tree, &DirectPath, &mut rng);

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

    let result = brain.tick_with_behavior_tree(16.0, &[], &tree, &DirectPath, &mut rng);

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

    let result = brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);

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

    brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);
    assert_eq!(brain.state(), AiState::Flee);

    // Large delta covers the whole flee leg: monster ends at x=40,
    // 32m from the attacker — beyond the 30m safe distance.
    brain.tick_with_behavior_tree(5000.0, &players, &tree, &DirectPath, &mut rng);

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
    brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);
    assert_eq!(brain.state(), AiState::Flee);

    // Attacker chased to x=35 — when the first leg ends at x=40 the
    // monster is still within sight, so it must start another leg east.
    let chasing = attacker_at(35.0, 10.0);
    let result = brain.tick_with_behavior_tree(5000.0, &chasing, &tree, &DirectPath, &mut rng);

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

    let result = brain.tick_with_behavior_tree(16.0, &[], &tree, &DirectPath, &mut rng);

    assert_eq!(brain.state(), AiState::Idle);
    assert!(result.commands.is_empty());
}

#[test]
fn behavior_tree_return_sends_walk_target_to_spawn() {
    let mut brain = make_brain();
    brain.position.x = 70.0;
    let tree = BehaviorTree {
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
    };
    let mut rng = SmallRng::seed_from_u64(42);

    let result = brain.tick_with_behavior_tree(16.0, &[], &tree, &DirectPath, &mut rng);

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

    let peaceful = brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);
    assert!(!peaceful
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));
    assert_eq!(brain.state(), AiState::Idle);

    brain.handle_hit_with_behavior_tree(&1.into(), false, 0);
    let provoked = brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);
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

    let result = brain.tick_with_behavior_tree(50.0, &players, &tree, &DirectPath, &mut rng);
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
        brain.tick_with_behavior_tree(1700.0, &players, &tree, &DirectPath, &mut rng);
    assert!(!before_cooldown
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));

    let after_cooldown =
        brain.tick_with_behavior_tree(100.0, &players, &tree, &DirectPath, &mut rng);
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
        let result = brain.tick_with_behavior_tree(16.0, players, &tree, &DirectPath, &mut rng);
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
    holding.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);
    assert_eq!(
        holding.state(),
        AiState::Attack,
        "an attack must not drop the frame the target steps outside it"
    );

    let mut engaging = make_brain();
    engaging.attack_range = 2.0;
    engaging.target_player_id = Some(1.into());
    engaging.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);
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
    let result = brain.tick_with_behavior_tree(16.0, &near, &tree, &DirectPath, &mut rng);
    assert!(result
        .commands
        .iter()
        .any(|c| matches!(c, AiCommand::Attack { .. })));

    // The target sprints clear, but the swing is only one frame old.
    let far = attacker_at(20.0, 10.0);
    brain.tick_with_behavior_tree(16.0, &far, &tree, &DirectPath, &mut rng);
    assert_eq!(
        brain.state(),
        AiState::Attack,
        "the swing must land before the monster gives it up"
    );

    brain.tick_with_behavior_tree(450.0, &far, &tree, &DirectPath, &mut rng);
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
        let result = brain.tick_with_behavior_tree(16.0, &players, &tree, &BentPath, &mut rng);
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

    let corner = Position {
        x: 30.0,
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
        brain.tick_with_behavior_tree(16.0, &players, &tree, &BentPath, &mut rng);
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

    brain.tick_with_behavior_tree(16.0, &players, &tree, &BentPath, &mut rng);

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
        brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);
    }
    assert_eq!(brain.state(), AiState::Flee);

    let kept = Position {
        x: 20.0,
        y: 0.0,
        z: 10.0,
    };
    brain.apply_authoritative_position(kept);
    brain.tick_with_behavior_tree(16.0, &players, &tree, &DirectPath, &mut rng);

    assert_eq!(
        brain.state(),
        AiState::Flee,
        "a correction must not end the flee"
    );
    assert!(!brain.waypoints.is_empty(), "the brain must have repathed");
}

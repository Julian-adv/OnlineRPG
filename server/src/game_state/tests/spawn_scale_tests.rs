//! Scale measurements for the ambient spawn path, sized against the 5,000
//! concurrent-user target. Run with --nocapture to read the timings.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const USERS: usize = 5_000;

/// Fill the monster map directly — spawn_monster itself is what we measure.
/// `place` picks each monster's position from its index and its owner.
async fn preload_monsters(
    game_state: &GameState,
    count: usize,
    owners: &[PlayerId],
    place: impl Fn(usize, &PlayerId) -> Position,
) {
    let mut monsters = game_state.monsters.write().await;
    for i in 0..count {
        let owner = owners[i % owners.len()];
        let mut monster = make_monster(&format!("pre{i}"), place(i, &owner), 0);
        monster.owner_id = Some(owner);
        monster.monster_type = "goblin".to_string();
        monsters.insert(monster.id.clone(), monster);
    }
}

/// The grid the spawn measurements use: monsters spread across the world,
/// unrelated to where their owners stand.
fn grid_position(index: usize, _owner: &PlayerId) -> Position {
    Position {
        x: (index % 2000) as f32 * 3.0,
        y: 0.0,
        z: (index / 2000) as f32 * 3.0,
    }
}

/// Where each owner currently stands, for the measurements that put every
/// monster at its owner's feet. Read up front because `preload_monsters`'
/// placement closure runs under the registry write guard and cannot await.
async fn owner_positions(
    game_state: &GameState,
    owners: &[PlayerId],
) -> std::collections::HashMap<PlayerId, Position> {
    let players = game_state.players.read().await;
    owners
        .iter()
        .map(|id| (*id, players[id].position))
        .collect()
}

async fn add_bots(game_state: &GameState, count: usize) -> Vec<PlayerId> {
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let name = format!("scale_bot{i}");
        let id = pid(&name);
        let player = make_player(&name, (i % 100) as f32 * 60.0, (i / 100) as f32 * 60.0);
        game_state.add_player(player).await;
        ids.push(id);
    }
    ids
}

#[tokio::test]
#[ignore = "measurement, not an assertion; run explicitly with --nocapture"]
async fn spawn_path_cost_at_scale() {
    // Populations spanning the target scale; the measured spawns below run
    // the full insert path at each.
    for &population in &[1_000usize, 10_000, 50_000, 134_000] {
        let game_state = make_test_game_state(&format!("scale_{population}"));
        let owners = add_bots(&game_state, USERS).await;
        preload_monsters(&game_state, population, &owners, grid_position).await;

        let spawner = owners[0];
        let start = Instant::now();
        const SPAWNS: usize = 50;
        for i in 0..SPAWNS {
            game_state
                .spawn_monster(
                    "goblin".to_string(),
                    Position {
                        x: 5.0 + i as f32,
                        y: 0.0,
                        z: 5.0,
                    },
                    0.0,
                    Some(spawner),
                    -1,
                    // Slot lifecycle skips the per-player cap, so all 50
                    // spawns succeed and we time the real work.
                    MonsterLifecycle::DungeonSlot,
                    None,
                    false,
                )
                .await
                .expect("dungeon-slot spawns skip the per-player cap");
        }
        let per_spawn = start.elapsed() / SPAWNS as u32;

        let start = Instant::now();
        game_state.tick_monster_spawns().await;
        let spawn_tick = start.elapsed();

        // First tick reconciles every stale owner at once — a cold-start
        // transient. The second is the steady state: owners already correct,
        // so each monster settles as soon as its own owner turns up.
        let start = Instant::now();
        game_state.tick_monster_ownership().await;
        let cold_tick = start.elapsed();
        let start = Instant::now();
        game_state.tick_monster_ownership().await;
        let warm_tick = start.elapsed();

        // The per-move fanout is the steady-state cost: every alive monster
        // reports movement roughly once a second.
        let movers: Vec<(PlayerId, String, Position)> = {
            let monsters = game_state.monsters.read().await;
            monsters
                .values()
                .filter(|m| m.owner_id.is_some())
                .take(200)
                .map(|m| (m.owner_id.unwrap(), m.id.clone(), m.position))
                .collect()
        };
        let start = Instant::now();
        for (owner, id, position) in &movers {
            let target = Position {
                x: position.x + 0.5,
                ..*position
            };
            game_state
                .update_monster_position(owner, id.clone(), target, 0.0, MonsterState::Idle, target)
                .await;
        }
        let per_move = start.elapsed() / movers.len() as u32;

        println!(
            "{population:>7} monsters / {USERS} users: spawn_monster {per_spawn:>10.2?}  \
             tick_monster_spawns {spawn_tick:>10.2?}  ownership cold {cold_tick:>10.2?} \
             warm {warm_tick:>10.2?}  \
             move {per_move:>10.2?}"
        );
    }
}

/// The hottest read of the registry: every accepted move packet diffs the
/// mover's monster AOI, so this runs thousands of times a second at target
/// population — far more often than any background tick.
///
/// Three movers, all against the same 134k monsters: one in a crowd sharing its
/// cell with 200 other players' monsters (the pessimistic case), one standing on
/// its own ~27 with the nearest neighbours 60m out, and one nowhere near a
/// monster. Each oscillates in place so AOI membership never actually changes:
/// the timing covers the whole fanout — the player-side diff included — but no
/// spawn or removal sends, which would otherwise swamp the crowd figure.
#[tokio::test]
#[ignore = "measurement, not an assertion; run explicitly with --nocapture"]
async fn player_move_fanout_cost_at_scale() {
    const POPULATION: usize = 134_000;
    /// Players packed into one spot, monsters and all.
    const CROWD: usize = 200;
    let crowd_spot = Position {
        x: 9_000.0,
        y: 0.0,
        z: 9_000.0,
    };

    let game_state = make_test_game_state("player_move_fanout");
    let owners = add_bots(&game_state, USERS).await;
    for owner in owners.iter().take(CROWD) {
        game_state.teleport_player(owner, crowd_spot, 0.0, 0).await;
    }
    // Read after the teleports, so a crowd member's monsters land in the crowd
    // with it.
    let positions = owner_positions(&game_state, &owners).await;
    preload_monsters(&game_state, POPULATION, &owners, |_, owner| {
        positions[owner]
    })
    .await;

    let sparse = pid("sparse_mover");
    game_state
        .add_player(make_player("sparse_mover", 12_000.0, 12_000.0))
        .await;

    for (label, mover) in [
        ("crowd", owners[0]),
        ("own monsters", owners[CROWD]),
        ("no monsters", sparse),
    ] {
        let origin = game_state.players.read().await[&mover].position;
        const MOVES: usize = 200;
        let start = Instant::now();
        for i in 0..MOVES {
            let position = Position {
                x: origin.x + if i % 2 == 0 { 0.25 } else { -0.25 },
                ..origin
            };
            game_state
                .update_player_position(&mover, move_cmd(position, false), true, false)
                .await;
        }
        let per_move = start.elapsed() / MOVES as u32;
        println!(
            "{POPULATION} monsters / {USERS} users: player move ({label:>12}) {per_move:>10.2?}"
        );
    }
}

/// The ownership tick's own runtime is fine on a 10s timer; what matters is how
/// long it keeps the registry from the writers — every kill and every monster
/// move takes `monsters.write()`. Every monster sits on its owner here, so the
/// tick is pure scan with no handoff or despawn writes of its own, and the
/// reported wait is the scan's alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement, not an assertion; run explicitly with --nocapture"]
async fn ownership_tick_writer_stall() {
    const POPULATION: usize = 134_000;
    let game_state = make_test_game_state("ownership_stall");
    let owners = add_bots(&game_state, USERS).await;
    let positions = owner_positions(&game_state, &owners).await;
    preload_monsters(&game_state, POPULATION, &owners, |_, owner| {
        positions[owner]
    })
    .await;

    let stop = Arc::new(AtomicBool::new(false));
    let writer_state = game_state.clone();
    let writer_stop = Arc::clone(&stop);
    let writer = tokio::spawn(async move {
        let mut worst = Duration::ZERO;
        while !writer_stop.load(Ordering::Relaxed) {
            let start = Instant::now();
            let mut monsters = writer_state.monsters.write().await;
            worst = worst.max(start.elapsed());
            monsters.mark_dead("no_such_monster");
            drop(monsters);
            tokio::task::yield_now().await;
        }
        worst
    });

    let start = Instant::now();
    game_state.tick_monster_ownership().await;
    let tick = start.elapsed();
    stop.store(true, Ordering::Relaxed);
    let worst = writer.await.expect("writer task finishes");

    assert_eq!(
        game_state.monsters.read().await.len(),
        POPULATION,
        "every monster is attended by its owner, so none should be despawned"
    );
    println!(
        "{POPULATION} monsters / {USERS} users: ownership tick {tick:>10.2?}  \
         worst writer wait {worst:>10.2?}"
    );
}

/// What a disconnect costs. `remove_monsters_by_owner` runs on every socket
/// close, heartbeat timeout, duplicate login and /kick, and a network blip
/// bunches those into one 30s window. Times both branches: handed off to a
/// neighbour, and despawned with nobody around.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement, not an assertion; run explicitly with --nocapture"]
async fn disconnect_cleanup_cost_at_scale() {
    const POPULATION: usize = 134_000;
    let game_state = make_test_game_state("disconnect_cleanup");
    let owners = add_bots(&game_state, USERS).await;
    // Neighbour first, so it stands in the leaver's AOI with monsters of its
    // own — the load its adoption is weighed against.
    let leaver = owners[0];
    let neighbour = owners[1];
    let spot = game_state.players.read().await[&leaver].position;
    game_state.teleport_player(&neighbour, spot, 0.0, 0).await;
    let positions = owner_positions(&game_state, &owners).await;
    preload_monsters(&game_state, POPULATION, &owners, |_, owner| {
        positions[owner]
    })
    .await;

    let owned = game_state.monsters.read().await.owned_by(&leaver);
    let start = Instant::now();
    game_state.remove_monsters_by_owner(&leaver).await;
    let handoff_call = start.elapsed();

    // owners[2] is 60m from its nearest neighbour, so its monsters find no
    // adopter and take the despawn branch instead.
    let start = Instant::now();
    game_state.remove_monsters_by_owner(&owners[2]).await;
    let despawn_call = start.elapsed();

    assert_eq!(
        game_state.monsters.read().await.owned_by(&leaver),
        0,
        "the leaver keeps no monsters"
    );
    println!(
        "{POPULATION} monsters / {USERS} users: remove_monsters_by_owner \
         ({owned} owned) handoff {handoff_call:>10.2?} despawn {despawn_call:>10.2?}"
    );
}

/// The registry's owner index is the spawn cap and the disconnect cleanup, and
/// its cell index is what every AOI query walks. If either drifts from the map
/// the server silently over- or under-spawns, or shows the wrong monsters, so
/// pin both against a full scan after every kind of mutation.
#[tokio::test]
async fn registry_indexes_track_the_map_through_every_mutation() {
    let game_state = make_test_game_state("registry_indexes");
    let owners = add_bots(&game_state, 3).await;

    let mut spawned = Vec::new();
    for (i, owner) in owners.iter().enumerate() {
        for t in ["goblin", "orc"] {
            let monster = game_state
                .spawn_monster(
                    t.to_string(),
                    Position {
                        x: i as f32 * 10.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    0.0,
                    Some(*owner),
                    0,
                    MonsterLifecycle::Ambient,
                    None,
                    false,
                )
                .await
                .expect("spawn succeeds under the caps");
            spawned.push(monster.id);
        }
    }

    let audit = |label: &'static str| {
        let game_state = game_state.clone();
        async move {
            let monsters = game_state.monsters.read().await;
            assert!(
                monsters.cell_index_matches_map(),
                "{label}: the cell index drifted from the map"
            );
            assert!(
                monsters.owner_index_matches_map(),
                "{label}: the owner index drifted from the map"
            );
        }
    };

    audit("after spawns").await;

    // A move that changes cells, then one that does not — the second covers the
    // same-cell skip.
    let moved_to = Position {
        x: 900.0,
        y: 0.0,
        z: 900.0,
    };
    game_state
        .monsters
        .write()
        .await
        .set_position(&spawned[3], moved_to);
    audit("after a move across cells").await;
    game_state.monsters.write().await.set_position(
        &spawned[3],
        Position {
            x: 901.0,
            ..moved_to
        },
    );
    audit("after a move within one cell").await;

    // Inserting over a live key: the old owner has to be unfiled before the new
    // one is filed, or a same-owner replace drops the id it just re-filed.
    let mut replacement = game_state
        .monsters
        .read()
        .await
        .get(&spawned[3])
        .cloned()
        .expect("the monster is still in the map");
    replacement.owner_id = Some(owners[0]);
    game_state
        .monsters
        .write()
        .await
        .insert(spawned[3].clone(), replacement.clone());
    audit("after a replacing insert under a new owner").await;
    game_state
        .monsters
        .write()
        .await
        .insert(spawned[3].clone(), replacement);
    audit("after a replacing insert under the same owner").await;

    game_state.monsters.write().await.mark_dead(&spawned[0]);
    audit("after mark_dead").await;
    assert!(
        game_state
            .monsters
            .read()
            .await
            .ids_owned_by(&owners[0])
            .any(|id| id == spawned[0]),
        "a corpse stays filed under its owner: remove_monsters_by_owner clears it too"
    );
    // Idempotent: a second kill leaves the indexes alone.
    game_state.monsters.write().await.mark_dead(&spawned[0]);
    audit("after repeat mark_dead").await;

    game_state.monsters.write().await.remove(&spawned[0]);
    audit("after removing the corpse").await;

    game_state.monsters.write().await.remove(&spawned[1]);
    audit("after removing a live monster").await;

    game_state
        .monsters
        .write()
        .await
        .reassign_owner(&spawned[2], owners[2], 0);
    audit("after reassign_owner").await;

    // A corpse handed off, which is the disconnect path itself: `owned` below
    // carries dead monsters into `hand_off_monsters`.
    game_state.monsters.write().await.mark_dead(&spawned[4]);
    game_state
        .monsters
        .write()
        .await
        .reassign_owner(&spawned[4], owners[0], 0);
    audit("after reassigning a corpse").await;

    game_state.remove_monsters_by_owner(&owners[2]).await;
    audit("after owner disconnect").await;
}

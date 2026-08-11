//! Scale measurements for the ambient spawn path, sized against the 5,000
//! concurrent-user target. Run with --nocapture to read the timings.
use super::*;
use std::time::Instant;

const USERS: usize = 5_000;

/// Fill the monster map directly — spawn_monster itself is what we measure.
async fn preload_monsters(game_state: &GameState, count: usize, owners: &[PlayerId]) {
    let mut monsters = game_state.monsters.write().await;
    for i in 0..count {
        let owner = owners[i % owners.len()];
        let position = Position {
            x: (i % 2000) as f32 * 3.0,
            y: 0.0,
            z: (i / 2000) as f32 * 3.0,
        };
        let mut monster = make_monster(&format!("pre{i}"), position, 0);
        monster.owner_id = Some(owner);
        monster.monster_type = "goblin".to_string();
        monsters.insert(monster.id.clone(), monster);
    }
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
    // Kept just under the global cap so the measured spawns actually run the
    // full insert path instead of returning early on the limit.
    for &population in &[1_000usize, 10_000, 50_000, 134_000] {
        let game_state = make_test_game_state(&format!("scale_{population}"));
        let owners = add_bots(&game_state, USERS).await;
        preload_monsters(&game_state, population, &owners).await;

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
                    // Dungeon floor: skips the per-player cap, so all 50
                    // spawns succeed and we time the real work.
                    -1,
                    None,
                    false,
                )
                .await
                .expect("spawn succeeds below the global cap");
        }
        let per_spawn = start.elapsed() / SPAWNS as u32;

        let start = Instant::now();
        game_state.tick_monster_spawns().await;
        let spawn_tick = start.elapsed();

        let start = Instant::now();
        game_state.tick_abandoned_monsters().await;
        let abandon_tick = start.elapsed();

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
             tick_monster_spawns {spawn_tick:>10.2?}  tick_abandoned {abandon_tick:>10.2?}  \
             move {per_move:>10.2?}"
        );
    }
}

/// The registry's counts are the spawn caps. If they drift from the map the
/// server silently over- or under-spawns, so pin them against a full scan
/// after every kind of mutation.
#[tokio::test]
async fn registry_counts_track_the_map_through_every_mutation() {
    let game_state = make_test_game_state("registry_counts");
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
            let expected_total = monsters
                .values()
                .filter(|m| m.state != MonsterState::Dead)
                .count();
            assert_eq!(
                monsters.alive_total(),
                expected_total,
                "{label}: alive_total drifted from the map"
            );
            let mut expected_pairs = std::collections::HashMap::new();
            for m in monsters.values() {
                if m.state != MonsterState::Dead {
                    if let Some(owner) = m.owner_id {
                        *expected_pairs
                            .entry((owner, m.monster_type.clone()))
                            .or_insert(0u32) += 1;
                    }
                }
            }
            for ((owner, monster_type), count) in &expected_pairs {
                assert_eq!(
                    monsters.alive_for(owner, monster_type),
                    *count,
                    "{label}: per-owner count drifted for {monster_type}"
                );
            }
            assert_eq!(
                monsters.alive_by_owner_type_len(),
                expected_pairs.len(),
                "{label}: stale zero-count entries left behind"
            );
            let expected_owners: std::collections::HashSet<_> =
                expected_pairs.keys().map(|(owner, _)| *owner).collect();
            assert_eq!(
                monsters.alive_owner_count(),
                expected_owners.len(),
                "{label}: an owner with no alive monsters left an empty map behind"
            );
        }
    };

    audit("after spawns").await;

    game_state.monsters.write().await.mark_dead(&spawned[0]);
    audit("after mark_dead").await;
    // Idempotent: a second kill must not double-debit.
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
        .reassign_owner(&spawned[2], owners[2]);
    audit("after reassign_owner").await;

    game_state.remove_monsters_by_owner(&owners[2]).await;
    audit("after owner disconnect").await;
}

//! Soak harness: does ambient spawning still work after hours of roaming
//! agent-client bots? Compresses the 10s spawn tick (main.rs:445) into a loop.
use super::*;

const TICK_SECONDS: u64 = 10;
const TWO_HOURS_TICKS: u64 = 2 * 3600 / TICK_SECONDS;
/// How far a roaming bot travels between two spawn ticks (~3m/s for 10s).
const ROAM_PER_TICK: f32 = 30.0;
/// The bot chases and kills whatever it can see (NPC_SIGHT_RADIUS). Spawns
/// land at 22m, so a bot that stays put does clear its own spawns.
const KILL_RADIUS: f32 = onlinerpg_shared::NPC_SIGHT_RADIUS;

fn lcg(seed: &mut u64) -> f32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*seed >> 33) as f32) / (u32::MAX as f32 / 2.0)
}

/// Moves through the real API so `player_spatial_cells` stays in sync —
/// writing the roster directly leaves the proximity index stale.
async fn set_player_xz(game_state: &GameState, player_id: &PlayerId, x: f32, z: f32) {
    set_player_on_floor(game_state, player_id, x, z, 0).await;
}

async fn set_player_on_floor(
    game_state: &GameState,
    player_id: &PlayerId,
    x: f32,
    z: f32,
    floor: i8,
) {
    let position = Position {
        x: onlinerpg_shared::wrap_world_x(x),
        y: 0.0,
        z,
    };
    game_state
        .teleport_player(player_id, position, 0.0, floor)
        .await;
}

/// Mirrors connection.rs:1211 + agent-client's find_valid_spawn_position:
/// answer every SpawnMonsterRequest with a point 22m from the bot.
async fn answer_spawn_requests(
    game_state: &GameState,
    player_id: &PlayerId,
    rx: &mut DirectRx,
    seed: &mut u64,
) -> usize {
    let requested: Vec<String> = drain(rx)
        .into_iter()
        .filter_map(|msg| match msg {
            ServerMessage::SpawnMonsterRequest { monster_type } => Some(monster_type),
            _ => None,
        })
        .collect();

    let center = game_state.get_all_players().await[player_id].position;
    let mut spawned = 0;
    for monster_type in requested {
        let angle = lcg(seed) * std::f32::consts::TAU;
        let position = Position {
            x: onlinerpg_shared::wrap_world_x(center.x + angle.cos() * 22.0),
            y: 0.0,
            z: center.z + angle.sin() * 22.0,
        };
        let Some(position) = game_state
            .validate_spawn_request(player_id, &monster_type, &position, 0.0)
            .await
        else {
            continue;
        };
        if !game_state
            .take_spawn_allowance(player_id, &monster_type)
            .await
        {
            continue;
        }
        if game_state
            .spawn_monster(
                monster_type,
                position,
                0.0,
                Some(*player_id),
                0,
                None,
                false,
            )
            .await
            .is_some()
        {
            spawned += 1;
        }
    }
    spawned
}

/// The bot kills what is in reach; corpse cleanup (combat.rs:535) then removes
/// it. Monsters it has walked away from are left behind, exactly as in play.
async fn kill_monsters_in_reach(game_state: &GameState, player_id: &PlayerId) -> usize {
    let center = game_state.get_all_players().await[player_id].position;
    let mut monsters = game_state.monsters.write().await;
    let doomed: Vec<String> = monsters
        .values()
        .filter(|m| {
            m.owner_id.as_ref() == Some(player_id)
                && m.position.dist_xz_sq(&center) <= KILL_RADIUS * KILL_RADIUS
        })
        .map(|m| m.id.clone())
        .collect();
    for id in &doomed {
        monsters.remove(id);
    }
    doomed.len()
}

#[tokio::test]
async fn ambient_spawns_survive_two_hours_of_roaming_bots() {
    let game_state = make_test_game_state("spawn_soak");
    let player_id = pid("roaming_bot");
    game_state
        .add_player(make_player("roaming_bot", 0.0, 0.0))
        .await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    let mut seed = 0x5EED_1234u64;
    let mut heading = 0.0f32;
    let (mut x, mut z) = (0.0f32, 0.0f32);
    let mut spawns_by_tick = Vec::new();
    let mut kills_total = 0usize;

    for _ in 0..TWO_HOURS_TICKS {
        game_state.tick_monster_ownership().await;
        heading += (lcg(&mut seed) - 1.0) * 0.4;
        x += heading.cos() * ROAM_PER_TICK;
        z += heading.sin() * ROAM_PER_TICK;
        set_player_xz(&game_state, &player_id, x, z).await;

        // Kill before spawning: a real kill costs a chase plus several swings,
        // so a monster spawned this tick is only reachable on a later one.
        kills_total += kill_monsters_in_reach(&game_state, &player_id).await;
        game_state.tick_monster_spawns().await;
        spawns_by_tick
            .push(answer_spawn_requests(&game_state, &player_id, &mut rx, &mut seed).await);
    }

    let alive = game_state.monsters.read().await.len();
    let first_hour: usize = spawns_by_tick[..spawns_by_tick.len() / 2].iter().sum();
    let last_30: usize = spawns_by_tick[spawns_by_tick.len() - 30..].iter().sum();
    println!(
        "spawned in hour 1: {first_hour}, in last 5 min: {last_30}, killed: {kills_total}, alive at end: {alive}"
    );

    assert!(
        last_30 > 0,
        "no ambient monster spawned in the last 5 minutes after 2h of roaming \
         (hour 1 spawned {first_hour}, {alive} monsters still alive)"
    );
}

/// H2 at scale: monsters roaming bots left behind must not survive as
/// permanent cap-holders. Enough of them exhausts max_monsters_total and
/// starves every other player on the server.
#[tokio::test]
async fn abandoned_monsters_do_not_accumulate_across_many_bots() {
    let game_state = make_test_game_state("spawn_soak_global");
    let max_total = world_config().max_monsters_total as usize;
    let per_player: u32 = world_config()
        .ambient_spawns
        .iter()
        .map(|r| r.max_per_player)
        .sum();
    // Enough bots that unchecked abandonment would have exhausted the old
    // 1,000 cap; the current cap is sized for 5,000 users so it won't bind.
    let bots = 40.min(max_total / per_player as usize + 1);

    let mut bot_state = Vec::new();
    for i in 0..bots {
        let name = format!("bot{i}");
        let id = pid(&name);
        // Spread the bots far apart so nobody shares another's monsters.
        let (x, z) = (i as f32 * 500.0, i as f32 * 500.0);
        game_state.add_player(make_player(&name, x, z)).await;
        let rx = game_state.register_direct_channel(&id).await;
        bot_state.push((id, rx, x, z, 0.0f32));
    }

    let mut seed = 0xB07u64;
    for _ in 0..TWO_HOURS_TICKS {
        game_state.tick_monster_ownership().await;
        for (id, _, x, z, heading) in bot_state.iter_mut() {
            *heading += (lcg(&mut seed) - 1.0) * 0.4;
            *x += heading.cos() * ROAM_PER_TICK;
            *z += heading.sin() * ROAM_PER_TICK;
            set_player_xz(&game_state, id, *x, *z).await;
        }
        game_state.tick_monster_spawns().await;
        for (id, rx, ..) in bot_state.iter_mut() {
            answer_spawn_requests(&game_state, id, rx, &mut seed).await;
        }
    }

    let peak = game_state.monsters.read().await.len();
    let peak_unattended = count_unattended(&game_state).await;

    // Bots stop spawning and stand still. One tick clears everything they
    // walked away from — nothing unattended may remain.
    game_state.tick_monster_ownership().await;
    let alive = game_state.monsters.read().await.len();
    let unattended = count_unattended(&game_state).await;
    println!(
        "{bots} bots: peak {peak} alive ({peak_unattended} unattended, global cap \
         {max_total}) -> after drain {alive} alive ({unattended} unattended)"
    );

    assert_eq!(
        unattended, 0,
        "{unattended}/{alive} monsters have no player inside their AOI yet still \
         hold spawn-cap slots"
    );
}

/// Monsters no player is near — invisible to everyone, yet cap-consuming.
async fn count_unattended(game_state: &GameState) -> usize {
    let monsters = game_state.monsters.read().await;
    let mut unattended = 0;
    for monster in monsters.values() {
        if game_state
            .player_ids_within_position(
                &monster.position,
                monster.floor_level,
                EVENT_DELIVERY_RADIUS,
            )
            .await
            .is_empty()
        {
            unattended += 1;
        }
    }
    unattended
}

/// Control: same loop, bot never moves. If roaming is the load-bearing
/// element, this one keeps spawning forever.
#[tokio::test]
async fn stationary_bot_keeps_spawning() {
    let game_state = make_test_game_state("spawn_soak_still");
    let player_id = pid("still_bot");
    game_state
        .add_player(make_player("still_bot", 0.0, 0.0))
        .await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    let mut seed = 0x5EED_1234u64;
    let mut spawns_by_tick = Vec::new();
    for _ in 0..TWO_HOURS_TICKS {
        game_state.tick_monster_spawns().await;
        spawns_by_tick
            .push(answer_spawn_requests(&game_state, &player_id, &mut rx, &mut seed).await);
        kill_monsters_in_reach(&game_state, &player_id).await;
    }
    let last_30: usize = spawns_by_tick[spawns_by_tick.len() - 30..].iter().sum();
    println!("stationary spawns in last 5 min: {last_30}");
    assert!(last_30 > 0);
}

/// Minimal repro of the bug, inverted into its fix: no kills, no roaming loop,
/// one monster type. Fill the per-player cap and walk 1km away — before the
/// despawn sweep the abandoned goblins held the cap forever and the server
/// stopped asking; now the slot comes back.
#[tokio::test]
async fn walking_away_frees_the_cap_for_a_new_spawn_request() {
    let game_state = make_test_game_state("spawn_soak_min");
    let player_id = pid("walker");
    game_state.add_player(make_player("walker", 0.0, 0.0)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    let cap = world_config()
        .ambient_spawns
        .iter()
        .find(|r| r.monster_type == "goblin")
        .unwrap()
        .max_per_player;
    let mut seed = 1;
    for _ in 0..cap {
        game_state.tick_monster_spawns().await;
        answer_spawn_requests(&game_state, &player_id, &mut rx, &mut seed).await;
    }

    set_player_xz(&game_state, &player_id, 1000.0, 1000.0).await;
    game_state.tick_monster_ownership().await;
    game_state.tick_monster_spawns().await;

    assert_eq!(
        spawn_requests(&mut rx, "goblin"),
        1,
        "the despawn sweep freed the cap, so the walker is owed a goblin request"
    );
}

/// Sets up one monster on `floor`, its owner far away, and a second player
/// standing next to it. Returns (monster id, owner, owner rx, adopter, adopter rx).
async fn abandoned_next_to_a_bystander(
    game_state: &GameState,
    floor: i8,
) -> (String, PlayerId, DirectRx, PlayerId, DirectRx) {
    let (owner, adopter) = (pid("leaver"), pid("stayer"));
    game_state.add_player(make_player("leaver", 0.0, 0.0)).await;
    game_state.add_player(make_player("stayer", 0.0, 0.0)).await;
    set_player_on_floor(game_state, &owner, 0.0, 0.0, floor).await;
    set_player_on_floor(game_state, &adopter, 0.0, 0.0, floor).await;

    let monster = game_state
        .spawn_monster(
            "goblin".to_string(),
            Position {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
            Some(owner),
            floor,
            None,
            false,
        )
        .await
        .expect("spawn succeeds under the caps");

    // The owner walks out of the monster's AOI; the bystander stays put.
    set_player_on_floor(game_state, &owner, 1000.0, 1000.0, floor).await;

    let mut owner_rx = game_state.register_direct_channel(&owner).await;
    let mut adopter_rx = game_state.register_direct_channel(&adopter).await;
    drain(&mut owner_rx);
    drain(&mut adopter_rx);
    (monster.id, owner, owner_rx, adopter, adopter_rx)
}

async fn owner_of(game_state: &GameState, monster_id: &str) -> Option<PlayerId> {
    game_state
        .monsters
        .read()
        .await
        .get(monster_id)
        .and_then(|m| m.owner_id)
}

/// The owner's client is what simulates a monster, and it drops the brain the
/// moment the monster leaves its AOI. Left alone, the monster stays visible and
/// attackable to the bystander while nothing drives it.
#[tokio::test]
async fn a_monster_its_owner_left_is_handed_to_a_player_still_beside_it() {
    let game_state = make_test_game_state("handoff_surface");
    let (monster_id, _, mut owner_rx, adopter, mut adopter_rx) =
        abandoned_next_to_a_bystander(&game_state, 0).await;

    game_state.tick_monster_ownership().await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(adopter),
        "the bystander should have been given the monster"
    );
    assert!(
        drain(&mut adopter_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterAssigned { monster } if monster.id == monster_id)
        ),
        "the new owner needs MonsterAssigned to start running its AI"
    );
    assert!(
        drain(&mut owner_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterRemoved { monster_id: id } if *id == monster_id)
        ),
        "the old owner must be told it no longer holds the monster"
    );
}

/// A dungeon floor is 80m across but the AOI is 43m, so an owner can leave its
/// monster's AOI without leaving the floor — and the floor-exit handoff never
/// fires. This is the case that made dungeon monsters into punching bags.
#[tokio::test]
async fn handoff_also_covers_a_dungeon_floor() {
    let game_state = make_test_game_state("handoff_dungeon");
    let (monster_id, _, _, adopter, _) = abandoned_next_to_a_bystander(&game_state, -1).await;

    game_state.tick_monster_ownership().await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(adopter),
        "a dungeon monster whose owner walked to the far side of the floor should change hands"
    );
}

/// Handoff and despawn are separate halves: the dungeon's own floor lifecycle
/// owns removal, so the sweep must never delete a dungeon monster.
#[tokio::test]
async fn the_sweep_never_despawns_a_dungeon_monster() {
    let game_state = make_test_game_state("handoff_dungeon_keep");
    let owner = pid("delver");
    game_state.add_player(make_player("delver", 0.0, 0.0)).await;
    set_player_on_floor(&game_state, &owner, 0.0, 0.0, -1).await;
    let monster = game_state
        .spawn_monster(
            "goblin".to_string(),
            Position {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
            Some(owner),
            -1,
            None,
            false,
        )
        .await
        .expect("spawn succeeds under the caps");
    // Nobody anywhere near it.
    set_player_on_floor(&game_state, &owner, 1000.0, 1000.0, -1).await;
    game_state.tick_monster_ownership().await;

    assert!(
        game_state.monsters.read().await.get(&monster.id).is_some(),
        "the dungeon floor lifecycle owns this monster, not the abandonment sweep"
    );
}

/// The owner is still standing there: nothing to reconcile.
#[tokio::test]
async fn a_monster_whose_owner_is_present_is_left_alone() {
    let game_state = make_test_game_state("handoff_noop");
    let owner = pid("present");
    game_state
        .add_player(make_player("present", 0.0, 0.0))
        .await;
    let monster = game_state
        .spawn_monster(
            "goblin".to_string(),
            Position {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
            Some(owner),
            0,
            None,
            false,
        )
        .await
        .expect("spawn succeeds under the caps");
    let mut rx = game_state.register_direct_channel(&owner).await;
    drain(&mut rx);

    game_state.tick_monster_ownership().await;

    assert_eq!(owner_of(&game_state, &monster.id).await, Some(owner));
    assert!(
        drain(&mut rx).is_empty(),
        "an untouched monster should generate no ownership traffic"
    );
}

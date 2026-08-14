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

fn requested_types(rx: &mut DirectRx) -> Vec<String> {
    drain(rx)
        .into_iter()
        .filter_map(|message| match message {
            ServerMessage::SpawnMonsterRequest { monster_type } => Some(monster_type),
            _ => None,
        })
        .collect()
}

/// Mirrors connection.rs:1211 + agent-client's find_valid_spawn_position:
/// answer every SpawnMonsterRequest with a point 22m from the bot.
async fn answer_spawn_requests(
    game_state: &GameState,
    player_id: &PlayerId,
    rx: &mut DirectRx,
    seed: &mut u64,
) -> usize {
    let requested = requested_types(rx);

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
                MonsterLifecycle::Ambient,
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
/// permanent cap-holders. Each one holds a slot in its owner's cap, and since
/// that cap is now the only bound on the server's monster count, abandoned
/// monsters accumulating is what would let it grow without limit.
#[tokio::test]
async fn abandoned_monsters_do_not_accumulate_across_many_bots() {
    let game_state = make_test_game_state("spawn_soak_global");
    let per_player = world_config().max_monsters_per_player as usize;
    let bots = 40;

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
        "{bots} bots x {per_player}/player: peak {peak} alive ({peak_unattended} \
         unattended) -> after drain {alive} alive ({unattended} unattended)"
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

/// Minimal repro of the original bug, inverted into its fix: no kills, no
/// roaming loop. Fill the per-player cap and walk 1km away — abandoned
/// monsters once held the cap forever and the server stopped asking; now the
/// walk itself frees the slots, no sweep needed.
#[tokio::test]
async fn walking_away_frees_the_cap_for_a_new_spawn_request() {
    let game_state = make_test_game_state("spawn_soak_min");
    let player_id = pid("walker");
    game_state.add_player(make_player("walker", 0.0, 0.0)).await;
    let mut rx = game_state.register_direct_channel(&player_id).await;

    let cap = world_config().max_monsters_per_player as usize;
    let mut seed = 1;
    // One request per type a tick, so filling the cap takes a few rounds.
    for _ in 0..cap {
        if game_state.monsters.read().await.owned_by(&player_id) >= cap {
            break;
        }
        game_state.tick_monster_spawns().await;
        answer_spawn_requests(&game_state, &player_id, &mut rx, &mut seed).await;
    }
    assert_eq!(
        game_state.monsters.read().await.owned_by(&player_id),
        cap,
        "the walker should be at its cap before it walks away"
    );

    // Full: the server has nothing to offer.
    game_state.tick_monster_spawns().await;
    assert_eq!(
        spawn_requests(&mut rx, "goblin"),
        0,
        "a player at its cap must not be asked for another monster"
    );

    set_player_xz(&game_state, &player_id, 1000.0, 1000.0).await;
    game_state.tick_monster_spawns().await;

    assert_eq!(
        spawn_requests(&mut rx, "goblin"),
        1,
        "the walk released the abandoned monsters, so the walker is owed a goblin request"
    );
}

/// One goblin at (10, 0, 0) on `floor`, owned by `owner`.
async fn spawn_owned_goblin(
    game_state: &GameState,
    owner: PlayerId,
    floor: i8,
    lifecycle: MonsterLifecycle,
) -> String {
    game_state
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
            lifecycle,
            None,
            false,
        )
        .await
        .expect("spawn succeeds under the caps")
        .id
}

/// Puts `name` on `floor` at the origin and returns its id.
async fn add_player_on_floor(game_state: &GameState, name: &str, floor: i8) -> PlayerId {
    let id = pid(name);
    game_state.add_player(make_player(name, 0.0, 0.0)).await;
    set_player_on_floor(game_state, &id, 0.0, 0.0, floor).await;
    id
}

/// Sets up one monster on `floor` next to a second player, then walks the
/// owner out of its AOI — which, event-driven, is itself the handoff trigger.
async fn abandoned_next_to_a_bystander(
    game_state: &GameState,
    floor: i8,
    lifecycle: MonsterLifecycle,
) -> (String, DirectRx, PlayerId, DirectRx) {
    let owner = add_player_on_floor(game_state, "leaver", floor).await;
    let adopter = add_player_on_floor(game_state, "stayer", floor).await;
    let monster_id = spawn_owned_goblin(game_state, owner, floor, lifecycle).await;

    let mut owner_rx = game_state.register_direct_channel(&owner).await;
    let mut adopter_rx = game_state.register_direct_channel(&adopter).await;
    drain(&mut owner_rx);
    drain(&mut adopter_rx);

    // The owner walks out of the monster's AOI; the bystander stays put.
    set_player_on_floor(game_state, &owner, 1000.0, 1000.0, floor).await;
    (monster_id, owner_rx, adopter, adopter_rx)
}

/// The owner's client is what simulates a monster, and it drops the brain the
/// moment the monster leaves its AOI. The move that takes the owner out must
/// therefore also move the ownership — a sweep arriving seconds later would
/// leave the monster visible and attackable with nothing driving it.
#[tokio::test]
async fn walking_out_of_a_monsters_aoi_hands_it_off_on_the_spot() {
    let game_state = make_test_game_state("handoff_surface");
    let (monster_id, mut owner_rx, adopter, mut adopter_rx) =
        abandoned_next_to_a_bystander(&game_state, 0, MonsterLifecycle::Ambient).await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(adopter),
        "the walk itself should have handed the monster over — no sweep ran"
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
    let (monster_id, _, adopter, _) =
        abandoned_next_to_a_bystander(&game_state, -1, MonsterLifecycle::DungeonSlot).await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(adopter),
        "a dungeon monster whose owner walked to the far side of the floor should change hands"
    );
}

/// With nobody inside the monster's AOI there is no client to hand it to and
/// none that can see it: the same move that abandoned it despawns it, and the
/// owner is told so its brain dies with it.
#[tokio::test]
async fn walking_out_with_nobody_near_despawns_on_the_spot() {
    let game_state = make_test_game_state("event_despawn");
    let owner = add_player_on_floor(&game_state, "loner", 0).await;
    let monster_id = spawn_owned_goblin(&game_state, owner, 0, MonsterLifecycle::Ambient).await;
    let mut owner_rx = game_state.register_direct_channel(&owner).await;
    drain(&mut owner_rx);

    set_player_xz(&game_state, &owner, 1000.0, 1000.0).await;

    assert!(
        game_state.monsters.read().await.get(&monster_id).is_none(),
        "no player can see or simulate it: it should be gone without any sweep"
    );
    assert!(
        drain(&mut owner_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterRemoved { monster_id: id } if *id == monster_id)
        ),
        "the owner must be told to stop simulating it"
    );
}

/// A dungeon orphan is parked, not despawned — the floor lifecycle owns its
/// removal — and un-parked by the next player to walk into its AOI.
#[tokio::test]
async fn a_dungeon_orphan_parks_until_someone_walks_back_in() {
    let game_state = make_test_game_state("handoff_dungeon_keep");
    let owner = add_player_on_floor(&game_state, "delver", -1).await;
    let monster_id =
        spawn_owned_goblin(&game_state, owner, -1, MonsterLifecycle::DungeonSlot).await;
    let mut owner_rx = game_state.register_direct_channel(&owner).await;
    drain(&mut owner_rx);

    // Nobody anywhere near it.
    set_player_on_floor(&game_state, &owner, 1000.0, 1000.0, -1).await;

    assert!(
        game_state.monsters.read().await.get(&monster_id).is_some(),
        "the dungeon floor lifecycle owns this monster, not the abandonment path"
    );
    assert!(
        drain(&mut owner_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterRemoved { monster_id: id } if *id == monster_id)
        ),
        "parked still means the owner stops simulating it"
    );
    // The sweep must leave a parked dungeon monster alone too.
    game_state.tick_monster_ownership().await;
    assert!(
        game_state.monsters.read().await.get(&monster_id).is_some(),
        "the sweep must not despawn a dungeon monster either"
    );

    // A visitor walking into its AOI adopts it on sight.
    let visitor = pid("visitor");
    game_state
        .add_player(make_player("visitor", 1000.0, 0.0))
        .await;
    set_player_on_floor(&game_state, &visitor, 1000.0, 0.0, -1).await;
    let mut visitor_rx = game_state.register_direct_channel(&visitor).await;
    drain(&mut visitor_rx);
    set_player_on_floor(&game_state, &visitor, 10.0, 0.0, -1).await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(visitor),
        "walking into a parked monster's AOI should adopt it on sight"
    );
    assert!(
        drain(&mut visitor_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterAssigned { monster } if monster.id == monster_id)
        ),
        "the adopter needs MonsterAssigned to start running its AI"
    );
}

/// Lifecycle, not floor sign, decides the orphan policy: an ambient monster
/// on a dungeon floor (`/spawnmob` inside a dungeon) despawns when abandoned
/// with nobody near, instead of parking forever on a floor whose slots never
/// knew it.
#[tokio::test]
async fn an_ambient_monster_abandoned_in_a_dungeon_despawns_instead_of_parking() {
    let game_state = make_test_game_state("dungeon_ambient_despawn");
    let owner = add_player_on_floor(&game_state, "admin", -1).await;
    let monster_id = spawn_owned_goblin(&game_state, owner, -1, MonsterLifecycle::Ambient).await;

    set_player_on_floor(&game_state, &owner, 1000.0, 1000.0, -1).await;

    assert!(
        game_state.monsters.read().await.get(&monster_id).is_none(),
        "no slot owns it and nobody can see it — parking it would leak forever"
    );
}

/// The sweep keys the same policy off the tag: an unattended ambient monster
/// on a dungeon floor is despawned, not mistaken for a slot spawn.
#[tokio::test]
async fn the_sweep_despawns_an_unattended_ambient_monster_on_a_dungeon_floor() {
    let game_state = make_test_game_state("dungeon_ambient_sweep");
    let absent = add_player_on_floor(&game_state, "absent", -1).await;
    set_player_on_floor(&game_state, &absent, 2000.0, 2000.0, -1).await;
    let monster_id = spawn_owned_goblin(&game_state, absent, -1, MonsterLifecycle::Ambient).await;

    game_state.tick_monster_ownership().await;

    assert!(
        game_state.monsters.read().await.get(&monster_id).is_none(),
        "unattended and ambient: the sweep must despawn it despite the floor"
    );
}

/// Ownership only moves when the owner actually leaves, and the adopter is
/// inside the AOI by construction, so pacing across the boundary transfers
/// once and then goes quiet — the reason the event path needs no hysteresis.
#[tokio::test]
async fn pacing_across_the_boundary_does_not_bounce_ownership() {
    let game_state = make_test_game_state("boundary_pacing");
    let (monster_id, _owner_rx, adopter, mut adopter_rx) =
        abandoned_next_to_a_bystander(&game_state, 0, MonsterLifecycle::Ambient).await;
    assert_eq!(owner_of(&game_state, &monster_id).await, Some(adopter));
    drain(&mut adopter_rx);

    // The old owner paces back into the monster's AOI and out again.
    set_player_xz(&game_state, &pid("leaver"), 20.0, 0.0).await;
    set_player_xz(&game_state, &pid("leaver"), 1000.0, 1000.0).await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(adopter),
        "the adopter is still inside the AOI, so ownership must not move again"
    );
    let churn = drain(&mut adopter_rx)
        .into_iter()
        .filter(|m| {
            matches!(
                m,
                ServerMessage::MonsterAssigned { .. } | ServerMessage::MonsterRemoved { .. }
            )
        })
        .count();
    assert_eq!(
        churn, 0,
        "re-crossing the boundary must not re-negotiate ownership"
    );
}

/// Spawns a goblin at (40, 0, 0) — inside its owner's AOI — and banks enough
/// move budget that one update can carry it across the 43m boundary.
async fn goblin_near_the_edge(game_state: &GameState, owner: PlayerId) -> String {
    let monster_id = game_state
        .spawn_monster(
            "goblin".to_string(),
            Position {
                x: 40.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
            Some(owner),
            0,
            MonsterLifecycle::Ambient,
            None,
            false,
        )
        .await
        .expect("spawn succeeds under the caps")
        .id;
    let mut monsters = game_state.monsters.write().await;
    let monster = monsters.get_mut(&monster_id).expect("just spawned");
    monster.move_budget = 10.0;
    monster.last_move_at = GameState::now_ms();
    monster_id
}

/// Walks the monster from (40, 0) to (44.5, 0) as its owner's client and
/// asserts the move was accepted, so the ownership assertions that follow
/// actually saw a boundary crossing.
async fn wander_out(game_state: &GameState, owner: &PlayerId, monster_id: &str) {
    let walked_to = Position {
        x: 44.5,
        y: 0.0,
        z: 0.0,
    };
    game_state
        .update_monster_position(
            owner,
            monster_id.to_string(),
            walked_to,
            0.0,
            MonsterState::Run,
            walked_to,
        )
        .await;
    if let Some(monster) = game_state.monsters.read().await.get(monster_id) {
        assert_eq!(
            monster.position.x, walked_to.x,
            "the move must be accepted for this to test anything"
        );
    }
}

/// No player moved: the monster itself wandered out of its stationary owner's
/// AOI. The monster-move fanout is the only event that sees this, and it must
/// settle ownership the same way a player move does.
#[tokio::test]
async fn a_monster_that_wanders_off_is_handed_to_whoever_is_near_it() {
    let game_state = make_test_game_state("wander_handoff");
    let owner = add_player_on_floor(&game_state, "shepherd", 0).await;
    let stranger = add_player_on_floor(&game_state, "stranger", 0).await;
    set_player_xz(&game_state, &stranger, 80.0, 0.0).await;
    let monster_id = goblin_near_the_edge(&game_state, owner).await;

    wander_out(&game_state, &owner, &monster_id).await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(stranger),
        "44.5m is outside the shepherd's AOI and inside the stranger's"
    );
}

/// Same wander with nobody else in range: nobody can see it anymore, so it is
/// retired on the spot and the owner told to drop the brain that just
/// reported the move.
#[tokio::test]
async fn a_monster_that_wanders_out_of_everyones_sight_is_despawned() {
    let game_state = make_test_game_state("wander_despawn");
    let owner = add_player_on_floor(&game_state, "shepherd", 0).await;
    let monster_id = goblin_near_the_edge(&game_state, owner).await;
    let mut owner_rx = game_state.register_direct_channel(&owner).await;
    drain(&mut owner_rx);

    wander_out(&game_state, &owner, &monster_id).await;

    assert!(
        game_state.monsters.read().await.get(&monster_id).is_none(),
        "out of everyone's sight, the wanderer should be despawned"
    );
    assert!(
        drain(&mut owner_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterRemoved { monster_id: id } if *id == monster_id)
        ),
        "the owner must be told to stop simulating it"
    );
}

/// A monster stranded without its owner (spawned that way here, standing in
/// for the races the events can lose) is adopted by the first player whose
/// AOI it enters.
#[tokio::test]
async fn walking_up_to_a_stranded_monster_adopts_it_on_sight() {
    let game_state = make_test_game_state("adopt_on_sight");
    let absent = add_player_on_floor(&game_state, "absent", 0).await;
    set_player_xz(&game_state, &absent, 2000.0, 2000.0).await;
    let monster_id = spawn_owned_goblin(&game_state, absent, 0, MonsterLifecycle::Ambient).await;

    let walker = pid("walker");
    game_state
        .add_player(make_player("walker", 200.0, 0.0))
        .await;
    let mut walker_rx = game_state.register_direct_channel(&walker).await;
    set_player_xz(&game_state, &walker, 5.0, 0.0).await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(walker),
        "the walker found a monster nobody was simulating and should adopt it"
    );
    assert!(
        drain(&mut walker_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterAssigned { monster } if monster.id == monster_id)
        ),
        "the adopter needs MonsterAssigned to start running its AI"
    );
}

/// Every despawn — the 30s corpse sweep included — rides `despawn_monsters`,
/// whose announce reaches the owner directly: a monster whose owner is out of
/// range must not survive as a ghost on that owner's client.
#[tokio::test]
async fn despawning_tells_a_faraway_owner_directly() {
    let game_state = make_test_game_state("despawn_far_owner");
    let owner = add_player_on_floor(&game_state, "owner", 0).await;
    set_player_xz(&game_state, &owner, 2000.0, 2000.0).await;
    let monster_id = spawn_owned_goblin(&game_state, owner, 0, MonsterLifecycle::Ambient).await;
    let mut owner_rx = game_state.register_direct_channel(&owner).await;
    drain(&mut owner_rx);

    game_state.despawn_monsters(vec![monster_id.clone()]).await;

    assert!(
        drain(&mut owner_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterRemoved { monster_id: id } if *id == monster_id)
        ),
        "the owner is 2km away yet must still hear the removal"
    );
}

/// The sweep has no routine work left — events settle ownership as it changes
/// — but it must still repair a stranding they lost to a race. Forged here by
/// spawning the monster under an owner who is already far away, next to a
/// bystander who never moves (so no event ever fires).
#[tokio::test]
async fn the_sweep_still_repairs_a_stranding_the_events_missed() {
    let game_state = make_test_game_state("sweep_safety_net");
    let absent = add_player_on_floor(&game_state, "absent", 0).await;
    set_player_xz(&game_state, &absent, 2000.0, 2000.0).await;
    let bystander = add_player_on_floor(&game_state, "bystander", 0).await;
    let monster_id = spawn_owned_goblin(&game_state, absent, 0, MonsterLifecycle::Ambient).await;

    game_state.tick_monster_ownership().await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(bystander),
        "the sweep should hand the stranded monster to the bystander"
    );
}

/// The owner is still standing there: nothing to reconcile.
#[tokio::test]
async fn a_monster_whose_owner_is_present_is_left_alone() {
    let game_state = make_test_game_state("handoff_noop");
    let owner = add_player_on_floor(&game_state, "present", 0).await;
    let monster_id = spawn_owned_goblin(&game_state, owner, 0, MonsterLifecycle::Ambient).await;
    let mut rx = game_state.register_direct_channel(&owner).await;
    drain(&mut rx);

    game_state.tick_monster_ownership().await;

    assert_eq!(owner_of(&game_state, &monster_id).await, Some(owner));
    assert!(
        drain(&mut rx).is_empty(),
        "an untouched monster should generate no ownership traffic"
    );
}

/// Disconnecting is not different from walking away, as far as the people
/// standing next to the monster are concerned.
#[tokio::test]
async fn disconnecting_hands_monsters_to_players_still_beside_them() {
    let game_state = make_test_game_state("disconnect_handoff");
    let leaver = add_player_on_floor(&game_state, "leaver", 0).await;
    let stayer = add_player_on_floor(&game_state, "stayer", 0).await;
    let monster_id = spawn_owned_goblin(&game_state, leaver, 0, MonsterLifecycle::Ambient).await;
    let mut stayer_rx = game_state.register_direct_channel(&stayer).await;
    drain(&mut stayer_rx);

    game_state.remove_player(&leaver).await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(stayer),
        "the monster should have changed hands, not vanished from under the stayer"
    );
    assert!(
        drain(&mut stayer_rx).iter().any(
            |m| matches!(m, ServerMessage::MonsterAssigned { monster } if monster.id == monster_id)
        ),
        "the adopter needs MonsterAssigned to start running its AI"
    );
}

/// Nobody to adopt it, and the owner is not coming back.
#[tokio::test]
async fn disconnecting_alone_despawns_the_monsters() {
    let game_state = make_test_game_state("disconnect_alone");
    let leaver = add_player_on_floor(&game_state, "loner", 0).await;
    let monster_id = spawn_owned_goblin(&game_state, leaver, 0, MonsterLifecycle::Ambient).await;

    game_state.remove_player(&leaver).await;

    assert!(
        game_state.monsters.read().await.get(&monster_id).is_none(),
        "no client can ever simulate this monster again"
    );
}

/// One adopter should not inherit a whole cap's worth while others idle.
#[tokio::test]
async fn a_disconnected_players_monsters_are_spread_across_adopters() {
    let game_state = make_test_game_state("disconnect_spread");
    let leaver = add_player_on_floor(&game_state, "leaver", 0).await;
    let adopters = [
        add_player_on_floor(&game_state, "a", 0).await,
        add_player_on_floor(&game_state, "b", 0).await,
        add_player_on_floor(&game_state, "c", 0).await,
    ];
    // Distinct distances from the monsters at (10, 0, 0), all inside the AOI:
    // picking by distance alone would put every monster on `a`.
    for (adopter, x) in adopters.iter().zip([10.0, 25.0, 40.0]) {
        set_player_xz(&game_state, adopter, x, 0.0).await;
    }
    // Three, one per adopter: goblin's per-player cap is 5.
    for _ in 0..3 {
        spawn_owned_goblin(&game_state, leaver, 0, MonsterLifecycle::Ambient).await;
    }

    game_state.remove_player(&leaver).await;

    let owners: std::collections::HashSet<Option<PlayerId>> = game_state
        .monsters
        .read()
        .await
        .values()
        .map(|m| m.owner_id)
        .collect();
    let expected: std::collections::HashSet<Option<PlayerId>> =
        adopters.iter().map(|a| Some(*a)).collect();
    assert_eq!(
        owners, expected,
        "3 monsters over 3 adopters standing together should be one each"
    );
}

/// "Least loaded" has to mean least loaded overall, not merely least inherited
/// from this disconnect — otherwise a player already simulating a crowd looks
/// identical to an idle one and gets handed more.
#[tokio::test]
async fn a_busy_adopter_loses_to_an_idle_one_even_when_nearer() {
    let game_state = make_test_game_state("disconnect_load");
    let leaver = add_player_on_floor(&game_state, "leaver", 0).await;
    let busy = add_player_on_floor(&game_state, "busy", 0).await;
    let idle = add_player_on_floor(&game_state, "idle", 0).await;
    // Both inside the monster's AOI, but `busy` is sitting right on top of it.
    set_player_xz(&game_state, &busy, 10.0, 0.0).await;
    set_player_xz(&game_state, &idle, 30.0, 0.0).await;
    for _ in 0..3 {
        spawn_owned_goblin(&game_state, busy, 0, MonsterLifecycle::Ambient).await;
    }
    let monster_id = spawn_owned_goblin(&game_state, leaver, 0, MonsterLifecycle::Ambient).await;

    game_state.remove_player(&leaver).await;

    assert_eq!(
        owner_of(&game_state, &monster_id).await,
        Some(idle),
        "the idle player should adopt it despite being farther away"
    );
}

/// The walk-out release balances the same way a disconnect does: a scattering
/// party's monsters must not all land on whoever happens to be nearest.
#[tokio::test]
async fn abandoning_a_pair_spreads_them_across_bystanders() {
    let game_state = make_test_game_state("release_spread");
    let leaver = add_player_on_floor(&game_state, "leaver", 0).await;
    let near = add_player_on_floor(&game_state, "near", 0).await;
    let far = add_player_on_floor(&game_state, "far", 0).await;
    // Both bystanders inside the monsters' AOI, `near` closer to both.
    set_player_xz(&game_state, &near, 12.0, 0.0).await;
    set_player_xz(&game_state, &far, 30.0, 0.0).await;
    let a = spawn_owned_goblin(&game_state, leaver, 0, MonsterLifecycle::Ambient).await;
    let b = spawn_owned_goblin(&game_state, leaver, 0, MonsterLifecycle::Ambient).await;

    // The leaver walks away without disconnecting; the walk itself releases.
    set_player_xz(&game_state, &leaver, 1000.0, 1000.0).await;

    let owners: std::collections::HashSet<Option<PlayerId>> = [
        owner_of(&game_state, &a).await,
        owner_of(&game_state, &b).await,
    ]
    .into();
    assert_eq!(
        owners,
        [Some(near), Some(far)].into(),
        "two monsters over two bystanders should be one each, not both on the nearest"
    );
}

/// An ambient type only reaches players who have caught up to within one level
/// of it, so a beginner's surface stays at the monsters they can fight.
#[tokio::test]
async fn ambient_spawns_skip_players_below_the_monsters_level() {
    let game_state = make_test_game_state("ambient_level_gate");
    let gates: Vec<(String, u32)> = world_config()
        .ambient_spawns
        .iter()
        .map(|rule| {
            (
                rule.monster_type.clone(),
                game_state.min_ambient_player_level(&rule.monster_type),
            )
        })
        .collect();
    let highest = gates.iter().map(|(_, level)| *level).max().unwrap_or(0);
    assert!(
        highest > 1,
        "no ambient type outlevels a beginner, so this covers nothing"
    );

    let mut beginner = make_player("beginner", 0.0, 0.0);
    beginner.level = 1;
    let mut veteran = make_player("veteran", 200.0, 0.0);
    veteran.level = highest;
    game_state.add_player(beginner).await;
    game_state.add_player(veteran).await;
    let mut beginner_rx = game_state.register_direct_channel(&pid("beginner")).await;
    let mut veteran_rx = game_state.register_direct_channel(&pid("veteran")).await;

    game_state.tick_monster_spawns().await;

    let for_beginner = requested_types(&mut beginner_rx);
    let for_veteran = requested_types(&mut veteran_rx);
    for (monster_type, min_level) in &gates {
        assert_eq!(
            for_beginner.contains(monster_type),
            *min_level <= 1,
            "{monster_type} (needs level {min_level}) reached a level 1 player, or the types they \
             can fight did not"
        );
        assert!(
            for_veteran.contains(monster_type),
            "{monster_type} (needs level {min_level}) never reached a level {highest} player"
        );
    }
}

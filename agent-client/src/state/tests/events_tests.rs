use super::*;

/// The server never echoes our own monster moves back (the owner is
/// skipped in the fanout), so `send_command` must apply them locally.
#[tokio::test]
async fn outgoing_monster_move_echoes_into_local_state() {
    let (mut s, mut rx) = test_state();
    s.nearby_monsters.insert("m1".to_string(), monster("m1"));

    s.send_command(ClientMessage::MonsterMove {
        monster_id: "m1".to_string(),
        position: p(3.0, 1.0, 4.0),
        rotation: 1.5,
        state: MonsterState::Run,
        target_position: p(6.0, 1.0, 8.0),
    })
    .await
    .unwrap();

    let m = &s.nearby_monsters["m1"];
    assert_eq!(m.position.x, 3.0);
    assert_eq!(m.position.z, 4.0);
    assert_eq!(m.rotation, 1.5);
    assert_eq!(m.state, MonsterState::Run);

    match rx.try_recv() {
        Ok(ClientMessage::MonsterMove { monster_id, .. }) => assert_eq!(monster_id, "m1"),
        other => panic!("expected MonsterMove on the wire, got {other:?}"),
    }
}

/// A self-teleport must resync position, rotation AND floor, or the client
/// keeps walking from the stale spot and drags the character back.
#[test]
fn a_self_teleport_resyncs_position_and_floor() {
    let (mut s, _rx) = test_state();
    let me = test_player(-1464.5, 4690.5);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.self_floor_level = -5;

    s.push_event(ServerMessage::PlayerTeleported {
        player_id: PlayerId::from(1),
        position: p(-1456.0, 1.2, 4735.0),
        rotation: 2.5,
        floor_level: 0,
    });

    assert_eq!(
        s.self_floor_level, 0,
        "teleport must clear the stale dungeon floor"
    );
    let now = s.self_player.as_ref().unwrap();
    assert_eq!(now.position.x, -1456.0);
    assert_eq!(now.position.z, 4735.0);
    assert_eq!(now.rotation, 2.5);
    assert_eq!(now.floor_level, 0);
    assert_eq!(
        s.position_corrections, 1,
        "teleport must abandon any in-flight walk, like PositionCorrected"
    );
}

/// A respawn relocates us the same way, just via its own message.
#[test]
fn a_self_respawn_resyncs_position_and_floor() {
    let (mut s, _rx) = test_state();
    let me = test_player(-1464.5, 4690.5);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.self_floor_level = -5;

    let mut revived = test_player(-1475.0, 4742.0);
    revived.floor_level = 0;
    s.push_event(ServerMessage::PlayerRespawned { player: revived });

    assert_eq!(s.self_floor_level, 0);
    let now = s.self_player.as_ref().unwrap();
    assert_eq!(now.position.x, -1475.0);
    assert_eq!(now.floor_level, 0);
    assert_eq!(s.position_corrections, 1);
}

/// Someone else's teleport moves their tracked entry, not ours: mixing the
/// two would have us chase a neighbour's destination.
#[test]
fn a_neighbours_teleport_only_moves_their_entry() {
    let (mut s, _rx) = test_state();
    let me = test_player(-1464.5, 4690.5);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);

    let mut them = test_player(-1460.0, 4695.0);
    them.id = PlayerId::from(2);
    s.nearby_players.insert(them.id, them);

    s.push_event(ServerMessage::PlayerTeleported {
        player_id: PlayerId::from(2),
        position: p(-1456.0, 1.2, 4735.0),
        rotation: 0.0,
        floor_level: -3,
    });

    let them = &s.nearby_players[&PlayerId::from(2)];
    assert_eq!(them.position.x, -1456.0);
    assert_eq!(them.floor_level, -3);
    assert_eq!(s.self_player.as_ref().unwrap().position.x, -1464.5);
    assert_eq!(s.self_floor_level, 0);
    assert_eq!(
        s.position_corrections, 0,
        "a neighbour's teleport must not abandon our path"
    );
}

/// A dungeon monster's moves must keep its floor's height. Terrain snapping
/// would haul the whole floor's monsters up to the surface.
#[tokio::test]
async fn dungeon_monster_moves_keep_their_floor_height() {
    let (mut s, dungeon, _rx) = dungeon_state();
    let landing = dungeon.arrival_position(2).unwrap();
    let mut m = monster("m1");
    m.floor_level = -2;
    m.position = landing;
    s.nearby_monsters.insert("m1".to_string(), m);

    s.send_command(ClientMessage::MonsterMove {
        monster_id: "m1".to_string(),
        position: p(landing.x, 999.0, landing.z),
        rotation: 0.0,
        state: MonsterState::Run,
        target_position: p(landing.x, 999.0, landing.z),
    })
    .await
    .unwrap();

    let y = s.nearby_monsters["m1"].position.y;
    assert!(
        (y - dungeon.floor_y(2)).abs() < 0.01,
        "monster ended at y={y}, floor 2 sits at {}",
        dungeon.floor_y(2)
    );
}

/// Another player's FishingEnded renders no prompt line, so scheduling an
/// LLM cycle for it would buy a blank prompt; our own stays urgent.
#[test]
fn fishing_ended_wakes_llm_only_for_own_outcome() {
    let (mut s, _rx) = test_state();
    s.self_player_id = Some(PlayerId::from(1));
    let ended = |id: u64| ServerMessage::FishingEnded {
        player_id: PlayerId::from(id),
        outcome: onlinerpg_shared::fishing::FishingOutcome::Escaped,
    };
    assert_eq!(s.classify_event(&ended(1)), EventUrgency::Urgent);
    assert_eq!(s.classify_event(&ended(2)), EventUrgency::Noise);
}

/// The agent must not out-reflex a player at the same rod: the hook goes out
/// a human reaction later, not on the packet that carried the bite.
#[tokio::test(start_paused = true)]
async fn a_bite_is_hooked_after_a_human_delay() {
    use onlinerpg_shared::fishing::HOOK_REACTION_MS;
    let (mut s, mut rx) = test_state();
    s.self_player_id = Some(PlayerId::from(1));

    let t0 = tokio::time::Instant::now();
    s.push_event(ServerMessage::FishingBite {
        player_id: PlayerId::from(1),
    });
    assert!(rx.try_recv().is_err(), "the hook must not go out instantly");

    match rx.recv().await {
        Some(ClientMessage::FishingRespond { action }) => {
            assert_eq!(action, onlinerpg_shared::fishing::FishingAction::Hook)
        }
        other => panic!("expected a delayed hook, got {other:?}"),
    }
    let waited = t0.elapsed().as_millis() as u64;
    assert!(
        HOOK_REACTION_MS.contains(&waited),
        "hooked after {waited}ms, outside human range"
    );
}

/// A reaction still in the air when the session ends must never reach the
/// wire: the server reads a stray response during the next cast as the angler
/// yanking the rod, and the fish is gone.
#[tokio::test(start_paused = true)]
async fn the_session_ending_cancels_a_reaction_in_flight() {
    let (mut s, mut rx) = test_state();
    s.self_player_id = Some(PlayerId::from(1));

    s.push_event(ServerMessage::FishingBite {
        player_id: PlayerId::from(1),
    });
    s.push_event(ServerMessage::FishingEnded {
        player_id: PlayerId::from(1),
        outcome: onlinerpg_shared::fishing::FishingOutcome::Escaped,
    });

    tokio::time::sleep(std::time::Duration::from_millis(
        onlinerpg_shared::fishing::HOOK_REACTION_MS.end() + 1,
    ))
    .await;
    assert!(rx.try_recv().is_err(), "the stale hook must be cancelled");
}

/// One hand, one answer: a beat that lands mid-reaction is missed, and the
/// stance it would have taken is not recorded as if it had been sent.
#[tokio::test(start_paused = true)]
async fn a_beat_during_the_reaction_is_missed() {
    use onlinerpg_shared::fishing::{FishState, FishingAction};
    let (mut s, mut rx) = test_state();
    s.self_player_id = Some(PlayerId::from(1));
    let beat = |tension_pct| ServerMessage::FishingFight {
        player_id: PlayerId::from(1),
        bobber: p(0.0, 0.0, 0.0),
        fish_state: FishState::Resting,
        tension_pct,
        stamina_pct: 50,
    };

    s.push_event(beat(20));
    assert_eq!(s.fishing_stance, Some(FishingAction::Reel));
    s.push_event(beat(90));
    assert_eq!(
        s.fishing_stance,
        Some(FishingAction::Reel),
        "the gauge went red mid-reaction; the next beat answers it, not this one"
    );

    match rx.recv().await {
        Some(ClientMessage::FishingRespond { action }) => assert_eq!(action, FishingAction::Reel),
        other => panic!("expected the first stance, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "only one answer may be in flight");
}

/// The driver submits a prompt whenever the event buffer is non-empty, so
/// a spectator ending must skip the buffer entirely, not just rank low.
#[test]
fn fishing_ended_buffers_only_own_outcome() {
    let (mut s, _rx) = test_state();
    s.self_player_id = Some(PlayerId::from(1));
    let ended = |id: u64| ServerMessage::FishingEnded {
        player_id: PlayerId::from(id),
        outcome: onlinerpg_shared::fishing::FishingOutcome::Escaped,
    };
    s.push_event(ended(2));
    assert!(s.events.is_empty(), "spectator ending must not buffer");
    s.push_event(ended(1));
    assert_eq!(s.events.len(), 1, "own ending must reach the prompt");
}

#[test]
fn party_positions_do_not_wake_the_llm() {
    let (mut s, _rx) = test_state();
    let positions = ServerMessage::PartyPositions {
        members: Vec::new(),
    };
    assert_eq!(s.classify_event(&positions), EventUrgency::Noise);
    s.push_event(positions);
    assert!(s.events.is_empty());
}

/// An invalid_target rejection is the server saying the monster does not
/// exist; the stale entry must leave the list instead of being offered
/// to the LLM again next turn.
#[test]
fn invalid_target_rejection_drops_the_ghost_monster() {
    let (mut s, _rx) = test_state();
    s.nearby_monsters
        .insert("m_ghost".into(), monster("m_ghost"));
    s.push_event(ServerMessage::PlayerAttackRejected {
        monster_id: "m_ghost".into(),
        reason: onlinerpg_shared::AttackRejectReason::InvalidTarget,
    });
    assert!(!s.nearby_monsters.contains_key("m_ghost"));
    // Out-of-range rejections say nothing about existence.
    s.nearby_monsters.insert("m_far".into(), monster("m_far"));
    s.push_event(ServerMessage::PlayerAttackRejected {
        monster_id: "m_far".into(),
        reason: onlinerpg_shared::AttackRejectReason::OutOfRange,
    });
    assert!(s.nearby_monsters.contains_key("m_far"));
}

#[test]
fn quiet_grazers_get_no_sighting_event_but_hunters_do() {
    let (mut s, _rx) = test_state();
    s.self_player = Some(test_player(0.0, 0.0));
    let mut wolf = monster("m_wolf");
    wolf.aggressive = true;
    wolf.position = p(10.0, 0.0, 0.0);
    let mut slime = monster("m_slime");
    slime.position = p(-10.0, 0.0, 0.0);
    s.nearby_monsters.insert("m_wolf".into(), wolf);
    s.nearby_monsters.insert("m_slime".into(), slime);

    s.check_sightings();

    let sighted: Vec<&String> = s
        .agent_events
        .iter()
        .filter(|e| e.starts_with("[Sighted]"))
        .collect();
    assert_eq!(sighted.len(), 1, "only the aggressive monster: {sighted:?}");
    assert!(sighted[0].contains("m_wolf"));
    assert!(
        sighted[0].contains("at (10, 0), 10m east"),
        "sighting must carry coordinates and bearing: {}",
        sighted[0]
    );
}

/// A fresh drop (monster loot, chest ejection) arrives as
/// GroundItemSpawned; it must fire its sighting right away, not wait for
/// the next move to trigger a re-check.
#[test]
fn a_fresh_drop_is_sighted_immediately() {
    let (mut s, _rx) = test_state();
    s.self_player = Some(test_player(0.0, 0.0));
    s.push_event(ServerMessage::GroundItemSpawned {
        item: ground_item(77, "goblin_sword", 8.0, 0.0, 0),
    });
    assert!(
        s.agent_events
            .iter()
            .any(|e| e.starts_with("[Sighted]") && e.contains("goblin_sword")),
        "events: {:?}",
        s.agent_events
    );
}

/// Monster damage arrives only via MonsterAttackedPlayer/PlayerDead; both
/// must land in the local health mirrors, or a dead body reads as alive to
/// everything gated on it (auto-respawn, own-monster targeting).
#[test]
fn monster_damage_and_death_update_local_health() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    let mut neighbour = test_player(5.0, 5.0);
    neighbour.id = PlayerId::from(2);
    s.nearby_players.insert(neighbour.id, neighbour);

    s.push_event(ServerMessage::MonsterAttackedPlayer {
        monster_id: "m1".to_string(),
        player_id: PlayerId::from(1),
        hit: true,
        roll: 18,
        damage: 7,
        current_health: 3,
    });
    assert_eq!(s.self_player.as_ref().unwrap().health, 3);

    s.push_event(ServerMessage::MonsterAttackedPlayer {
        monster_id: "m1".to_string(),
        player_id: PlayerId::from(2),
        hit: true,
        roll: 18,
        damage: 6,
        current_health: 4,
    });
    assert_eq!(s.nearby_players[&PlayerId::from(2)].health, 4);

    s.push_event(ServerMessage::PlayerDead {
        player_id: PlayerId::from(1),
    });
    assert_eq!(s.self_player.as_ref().unwrap().health, 0);

    s.push_event(ServerMessage::PlayerDead {
        player_id: PlayerId::from(2),
    });
    assert_eq!(s.nearby_players[&PlayerId::from(2)].health, 0);
}

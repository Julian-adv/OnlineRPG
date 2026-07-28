use super::*;

/// Play the fight with a fixed stance policy until the session ends,
/// returning the outcome (plus every message seen on the way). The
/// policy sees each `FishingFight` beat's state and tension — exactly
/// what a real client (human gauge or agent reflex) gets.
async fn fight_to_the_end(
    game_state: &GameState,
    id: &PlayerId,
    rx: &mut UnboundedReceiver<ServerMessage>,
    policy: impl Fn(FishState, u32) -> FishingAction,
) -> (FishingOutcome, Vec<ServerMessage>) {
    let mut seen = Vec::new();
    // Budget: the fight timeout plus slack, in 250 ms ticks.
    for _ in 0..(FIGHT_TIMEOUT_MS / 250 + 40) {
        for msg in drain(rx) {
            match &msg {
                ServerMessage::FishingFight {
                    player_id,
                    fish_state,
                    tension_pct,
                    ..
                } if player_id == id => {
                    let action = policy(*fish_state, *tension_pct);
                    seen.push(msg.clone());
                    game_state.respond_fishing(id, action).await;
                }
                ServerMessage::FishingEnded { outcome, .. } => {
                    seen.push(msg.clone());
                    return (outcome.clone(), seen);
                }
                _ => seen.push(msg.clone()),
            }
        }
        advance(Duration::from_millis(250)).await;
        game_state.tick_fishing().await;
    }
    panic!("the fight never ended");
}

/// The wait roll is pure given an RNG: level 0 spans the shared range,
/// skill shortens it 2% per level, and the floor holds at half the minimum.
#[test]
fn wait_roll_shortens_with_skill_and_respects_the_floor() {
    use crate::game_state::fishing::roll_wait_ms;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    for seed in 0..50u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let base = roll_wait_ms(0, &mut rng);
        assert!(
            (u64::from(WAIT_MIN_MS)..=u64::from(WAIT_MAX_MS)).contains(&base),
            "level 0 wait {base} outside the shared range"
        );

        let mut rng = StdRng::seed_from_u64(seed);
        let skilled = roll_wait_ms(20, &mut rng);
        // Same seed → same base draw, shortened to exactly 60%.
        assert_eq!(skilled, base * 60 / 100, "level 20 = 40% shorter");

        let mut rng = StdRng::seed_from_u64(seed);
        assert_eq!(
            roll_wait_ms(50, &mut rng),
            u64::from(WAIT_MIN_MS) / 2,
            "an absurd level bottoms out at half the minimum wait"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn cast_requires_rod_water_and_range() {
    let game_state = make_test_game_state("fishing_cast_validation");
    let (id, mut rx) = make_angler(&game_state, "angler_val").await;

    // Land (positive x in the split world) is refused.
    game_state
        .start_fishing(
            &id,
            Position {
                x: 200.0,
                y: 0.0,
                z: 50.0,
            },
        )
        .await;
    // Out of range (>8m) is refused even over water.
    game_state
        .start_fishing(
            &id,
            Position {
                x: -150.0,
                y: 0.0,
                z: 50.0,
            },
        )
        .await;
    let errors = drain(&mut rx)
        .into_iter()
        .filter(|m| matches!(m, ServerMessage::FishingError { .. }))
        .count();
    assert_eq!(errors, 2);

    // No rod → refused.
    let bare = pid("angler_bare");
    game_state
        .add_player(make_player("angler_bare", -100.0, 50.0))
        .await;
    game_state.inventories.write().await.insert(
        bare,
        PlayerInventory {
            bag: vec![],
            equipped: Default::default(),
        },
    );
    let mut bare_rx = game_state.register_direct_channel(&bare).await;
    game_state.start_fishing(&bare, water_target()).await;
    assert!(drain(&mut bare_rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::FishingError { .. })));

    // Rod + water + in range → the cast broadcast reaches the angler.
    game_state.start_fishing(&id, water_target()).await;
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::FishingCasted { .. })));
}

// The regression this PR fixes: a river's bed sits ABOVE sea level (its
// carved channel bottoms out at sea level and climbs into the hills), so
// the old `terrain height < 0` water test rejected every inland river.
// With the unified water field, a river surface above its bed reads as
// water and the cast lands — end to end, all the way to a caught fish.
#[tokio::test(start_paused = true)]
async fn fishing_works_in_a_river_above_sea_level() {
    let game_state = make_river_game_state("fishing_river");
    // Plateau terrain is +5 m; the player and the water are both up there.
    let (id, mut rx) = make_angler(&game_state, "angler_river").await;

    game_state.start_fishing(&id, water_target()).await;
    let casted = drain(&mut rx);
    let bobber = casted.iter().find_map(|m| match m {
        ServerMessage::FishingCasted { position, .. } => Some(*position),
        _ => None,
    });
    let bobber = bobber.expect("river cast should be accepted, not refused as land");
    // The bobber floats on the river surface (~5.4 m), not at sea level.
    assert!(
        bobber.y > 5.0,
        "bobber should sit on the river surface, got y={}",
        bobber.y
    );

    // And the whole loop still resolves to a catch over the river.
    advance_until_bite(&game_state, &mut rx).await;
    game_state.respond_fishing(&id, FishingAction::Hook).await;
    let (outcome, _) = fight_to_the_end(&game_state, &id, &mut rx, auto_stance).await;
    assert!(
        matches!(outcome, FishingOutcome::Caught { .. }),
        "perfect play should land a river fish, got {outcome:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn full_catch_flow_awards_fish_and_skill_xp() {
    let game_state = make_test_game_state("fishing_catch_flow");
    let (id, mut rx) = make_angler(&game_state, "angler_catch").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_until_bite(&game_state, &mut rx).await;

    game_state.respond_fishing(&id, FishingAction::Hook).await;
    // Answer every struggle round correctly: the catch must land.
    let (outcome, msgs) = fight_to_the_end(&game_state, &id, &mut rx, auto_stance).await;
    let FishingOutcome::Caught {
        item_def_id: fish_id,
        size_cm,
        ..
    } = outcome
    else {
        panic!("perfect struggle play must catch, got {outcome:?}");
    };
    // The fight broadcast its beats, and the fish was visibly exhausted
    // before it could be landed.
    assert!(msgs
        .iter()
        .any(|m| matches!(m, ServerMessage::FishingFight { .. })));
    assert!(msgs.iter().any(|m| matches!(
        m,
        ServerMessage::FishingFight {
            fish_state: FishState::Exhausted,
            ..
        }
    )));
    assert!(size_cm > 0);
    // Every catch — fish, junk, and sealed coin pouches alike — lands in
    // the bag; only rarity ≥ 1 (fish) grants XP.
    let defs = ItemDefs::load();
    let def = defs.get(&fish_id).expect("caught def");
    let inv = game_state.get_player_inventory(&id).await.unwrap();
    assert!(inv
        .bag
        .iter()
        .any(|item| item.item_def_id == fish_id && item.quantity == 1));
    assert!(msgs
        .iter()
        .any(|m| matches!(m, ServerMessage::InventoryUpdated { .. })));
    let rarity = def.rarity_tier.unwrap_or(1);
    let got_xp = msgs.iter().any(|m| {
        matches!(
            m,
            ServerMessage::SkillXpGained { xp_amount, .. } if *xp_amount >= 10
        )
    });
    assert_eq!(
        got_xp,
        rarity >= 1,
        "fish grant XP, junk and coins do not (caught {fish_id}, rarity {rarity})"
    );
    // Session is gone: a second hook is an error, not a double catch.
    game_state.respond_fishing(&id, FishingAction::Hook).await;
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::FishingError { .. })));
}

#[tokio::test(start_paused = true)]
async fn ignored_bite_escapes_with_consolation_xp() {
    let game_state = make_test_game_state("fishing_bite_timeout");
    let (id, mut rx) = make_angler(&game_state, "angler_afk").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_until_bite(&game_state, &mut rx).await;

    // Sleep through the bite window plus both grace budgets.
    advance_with_ticks(
        &game_state,
        u64::from(BITE_WINDOW_MS + 2 * LATENCY_GRACE_MS + 500),
    )
    .await;
    let msgs = drain(&mut rx);
    assert!(msgs.iter().any(|m| matches!(
        m,
        ServerMessage::FishingEnded {
            outcome: FishingOutcome::Escaped,
            ..
        }
    )));
    assert!(msgs.iter().any(|m| matches!(
        m,
        ServerMessage::SkillXpGained { xp_amount, .. } if *xp_amount == 2
    )));
}

#[tokio::test(start_paused = true)]
async fn hooking_early_scares_the_fish_off_without_xp() {
    let game_state = make_test_game_state("fishing_early_hook");
    let (id, mut rx) = make_angler(&game_state, "angler_eager").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_with_ticks(&game_state, u64::from(CAST_MS) + 500).await;
    drain(&mut rx);

    game_state.respond_fishing(&id, FishingAction::Hook).await;
    let msgs = drain(&mut rx);
    assert!(msgs.iter().any(|m| matches!(
        m,
        ServerMessage::FishingEnded {
            outcome: FishingOutcome::Escaped,
            ..
        }
    )));
    assert!(!msgs
        .iter()
        .any(|m| matches!(m, ServerMessage::SkillXpGained { .. })));
}

#[tokio::test(start_paused = true)]
async fn moving_aborts_the_session() {
    let game_state = make_test_game_state("fishing_move_cancels");
    let (id, mut rx) = make_angler(&game_state, "angler_wanderer").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_with_ticks(&game_state, u64::from(CAST_MS) + 500).await;
    drain(&mut rx);

    // Any real relocation funnels through finish_position_update, which
    // breaks the session.
    game_state
        .update_player_position(
            &id,
            move_cmd(
                Position {
                    x: -95.0,
                    y: 0.0,
                    z: 50.0,
                },
                false,
            ),
            true,
            false,
        )
        .await;
    assert!(drain(&mut rx).iter().any(|m| matches!(
        m,
        ServerMessage::FishingEnded {
            outcome: FishingOutcome::Aborted,
            ..
        }
    )));
    // Idempotent: cancelling again stays quiet.
    game_state.cancel_fishing_if_active(&id).await;
    assert!(drain(&mut rx).is_empty());
}

/// A duplicated Hook (client double-send racing the fight open) must be
/// swallowed — not end the session, not change the stance.
#[tokio::test(start_paused = true)]
async fn duplicate_hook_during_the_fight_is_ignored() {
    let game_state = make_test_game_state("fishing_dup_hook");
    let (id, mut rx) = make_angler(&game_state, "angler_doublehook").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_until_bite(&game_state, &mut rx).await;
    game_state.respond_fishing(&id, FishingAction::Hook).await;
    assert!(drain(&mut rx)
        .iter()
        .any(|m| matches!(m, ServerMessage::FishingFight { .. })));

    game_state.respond_fishing(&id, FishingAction::Hook).await;
    assert!(
        !drain(&mut rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingEnded { .. })),
        "a duplicate hook must not end the fight"
    );

    // The fight is intact and still winnable.
    let (outcome, _) = fight_to_the_end(&game_state, &id, &mut rx, auto_stance).await;
    assert!(matches!(outcome, FishingOutcome::Caught { .. }));
}

/// Neither fish nor junk stack, so every bagged catch is its own slot even
/// when two of them are the same species — the bag reads as a catch log.
#[tokio::test(start_paused = true)]
async fn caught_fish_take_one_bag_slot_each() {
    let game_state = make_test_game_state("fishing_stacks");
    let (id, mut rx) = make_angler(&game_state, "angler_stacker").await;

    let mut bagged_catches = 0u32;
    for _ in 0..3 {
        game_state.start_fishing(&id, water_target()).await;
        advance_until_bite(&game_state, &mut rx).await;
        game_state.respond_fishing(&id, FishingAction::Hook).await;
        let (outcome, _) = fight_to_the_end(&game_state, &id, &mut rx, auto_stance).await;
        let FishingOutcome::Caught { .. } = outcome else {
            panic!("perfect play must catch");
        };
        // Every species takes a slot — even a coin pouch arrives sealed.
        bagged_catches += 1;
    }
    let inv = game_state.get_player_inventory(&id).await.unwrap();
    let total_fish: u32 = inv.bag.iter().map(|item| item.quantity).sum();
    assert_eq!(total_fish, bagged_catches);
    // Fish and junk are both non-stackable, so this holds no matter which
    // species the rolls produced.
    assert_eq!(
        inv.bag.len() as u32,
        bagged_catches,
        "each fish should occupy its own bag slot"
    );
    assert!(
        inv.bag.iter().all(|item| item.quantity == 1),
        "no fish entry should carry a quantity above one"
    );
}

// The fight: cranking the reel against a running fish pumps tension
// until the line snaps.
#[tokio::test(start_paused = true)]
async fn reeling_against_every_run_snaps_the_line() {
    let game_state = make_test_game_state("fishing_fight_wrong");
    let (id, mut rx) = make_angler(&game_state, "angler_clumsy").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_until_bite(&game_state, &mut rx).await;
    game_state.respond_fishing(&id, FishingAction::Hook).await;

    let (outcome, msgs) =
        fight_to_the_end(&game_state, &id, &mut rx, |_, _| FishingAction::Reel).await;
    assert_eq!(outcome, FishingOutcome::Escaped);
    // The snap came from tension, not the timeout: the fight died young.
    let beats = msgs
        .iter()
        .filter(|m| matches!(m, ServerMessage::FishingFight { .. }))
        .count();
    assert!(
        (beats as u32) * 250 < FIGHT_TIMEOUT_MS / 2,
        "reel-only play must snap early, saw {beats} beats"
    );
}

/// Never touching the rod can't land a fish: the line snaps or the fish
/// throws the hook at the timeout — either way it escapes (with the
/// consolation XP, since it was hooked).
#[tokio::test(start_paused = true)]
async fn ignoring_the_fight_escapes_the_fish() {
    let game_state = make_test_game_state("fishing_fight_afk");
    let (id, mut rx) = make_angler(&game_state, "angler_frozen").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_until_bite(&game_state, &mut rx).await;
    game_state.respond_fishing(&id, FishingAction::Hook).await;

    let mut msgs = Vec::new();
    let mut outcome = None;
    'outer: for _ in 0..(FIGHT_TIMEOUT_MS / 250 + 40) {
        for msg in drain(&mut rx) {
            if let ServerMessage::FishingEnded { outcome: o, .. } = &msg {
                outcome = Some(o.clone());
                msgs.push(msg);
                break 'outer;
            }
            msgs.push(msg);
        }
        advance(Duration::from_millis(250)).await;
        game_state.tick_fishing().await;
    }
    assert_eq!(outcome, Some(FishingOutcome::Escaped));
    assert!(msgs.iter().any(|m| matches!(
        m,
        ServerMessage::SkillXpGained { xp_amount, .. } if *xp_amount == ESCAPE_XP
    )));
}

// ---- PR9 hardening: concurrency, radius, aborts, overflow, consumption ----

/// Two sessions live in the same map at once; each angler's bites and rounds
/// are answered only by their owner, and both land their own catch with
/// their own XP. Locks the per-player session isolation.
#[tokio::test(start_paused = true)]
async fn two_anglers_fish_independently() {
    let game_state = make_test_game_state("fishing_two_anglers");
    let (a, mut rx_a) = make_angler(&game_state, "angler_left").await;
    let (b, mut rx_b) = make_angler(&game_state, "angler_right").await;

    game_state.start_fishing(&a, water_target()).await;
    game_state.start_fishing(&b, water_target()).await;

    let mut ended: std::collections::HashMap<PlayerId, FishingOutcome> = Default::default();
    let mut xp: std::collections::HashMap<PlayerId, u64> = Default::default();
    let mut caught: std::collections::HashMap<PlayerId, String> = Default::default();
    for _ in 0..400 {
        if ended.len() == 2 {
            break;
        }
        for (me, rx) in [(a, &mut rx_a), (b, &mut rx_b)] {
            for msg in drain(rx) {
                match msg {
                    ServerMessage::FishingBite { player_id } if player_id == me => {
                        game_state.respond_fishing(&me, FishingAction::Hook).await;
                    }
                    ServerMessage::FishingFight {
                        player_id,
                        fish_state,
                        tension_pct,
                        ..
                    } if player_id == me => {
                        game_state
                            .respond_fishing(&me, auto_stance(fish_state, tension_pct))
                            .await;
                    }
                    ServerMessage::SkillXpGained { xp_amount, .. } => {
                        *xp.entry(me).or_default() += xp_amount;
                    }
                    ServerMessage::FishingEnded { player_id, outcome } if player_id == me => {
                        if let FishingOutcome::Caught {
                            ref item_def_id, ..
                        } = outcome
                        {
                            caught.insert(me, item_def_id.clone());
                        }
                        ended.insert(me, outcome);
                    }
                    _ => {}
                }
            }
        }
        advance(Duration::from_millis(250)).await;
        game_state.tick_fishing().await;
    }

    assert_eq!(ended.len(), 2, "both anglers must finish their sessions");
    for (me, outcome) in &ended {
        assert!(
            matches!(outcome, FishingOutcome::Caught { .. }),
            "correct play must land the catch for {me}: {outcome:?}"
        );
        // XP mirrors the species each angler independently drew: fish grant
        // it, rarity-0 flotsam does not.
        let is_fish = game_state
            .item_defs
            .get(caught.get(me).expect("caught id"))
            .expect("caught def")
            .is_fish();
        assert_eq!(
            xp.get(me).copied().unwrap_or(0) > 0,
            is_fish,
            "skill XP must match what {me} caught ({:?})",
            caught.get(me)
        );
    }
}

/// Responding with no session gets a direct error and must not touch
/// anyone else's line.
#[tokio::test(start_paused = true)]
async fn responding_without_a_session_is_an_error_and_touches_nobody() {
    let game_state = make_test_game_state("fishing_kibitzer");
    let (a, mut rx_a) = make_angler(&game_state, "angler_focused").await;
    let b = pid("kibitzer");
    game_state
        .add_player(make_player("kibitzer", -100.0, 50.0))
        .await;
    let mut rx_b = game_state.register_direct_channel(&b).await;

    game_state.start_fishing(&a, water_target()).await;
    game_state.respond_fishing(&b, FishingAction::Hook).await;
    assert!(
        drain(&mut rx_b)
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingError { .. })),
        "the kibitzer is told they are not fishing"
    );

    // The angler's session is unaffected: bite, fight, catch as normal.
    advance_until_bite(&game_state, &mut rx_a).await;
    game_state.respond_fishing(&a, FishingAction::Hook).await;
    let (outcome, _) = fight_to_the_end(&game_state, &a, &mut rx_a, auto_stance).await;
    assert!(matches!(outcome, FishingOutcome::Caught { .. }));
}

/// Fishing broadcasts reach a bystander on the shore but not someone beyond
/// the delivery radius — bobbers render for neighbors, not the whole server.
#[tokio::test(start_paused = true)]
async fn fishing_broadcasts_are_radius_gated() {
    let game_state = make_test_game_state("fishing_radius");
    let (a, mut rx_a) = make_angler(&game_state, "angler_star").await;
    // Bobber lands at (-103, 50); 43 m delivery radius (shared::world).
    let near = pid("bystander_near");
    game_state
        .add_player(make_player("bystander_near", -90.0, 50.0))
        .await;
    let mut rx_near = game_state.register_direct_channel(&near).await;
    let far = pid("bystander_far");
    game_state
        .add_player(make_player("bystander_far", -40.0, 50.0))
        .await;
    let mut rx_far = game_state.register_direct_channel(&far).await;

    game_state.start_fishing(&a, water_target()).await;
    advance_until_bite(&game_state, &mut rx_a).await;
    game_state.respond_fishing(&a, FishingAction::Hook).await;
    let (outcome, _) = fight_to_the_end(&game_state, &a, &mut rx_a, auto_stance).await;
    assert!(matches!(outcome, FishingOutcome::Caught { .. }));

    fn fishing_msg_count(msgs: &[ServerMessage]) -> usize {
        msgs.iter()
            .filter(|m| {
                matches!(
                    m,
                    ServerMessage::FishingCasted { .. }
                        | ServerMessage::FishingBite { .. }
                        | ServerMessage::FishingFight { .. }
                        | ServerMessage::FishingEnded { .. }
                )
            })
            .count()
    }
    // The near bystander sees every phase — not just the cast. Asserting
    // each kind separately keeps a partial regression (e.g. the fight going
    // direct-only) from hiding behind the bobber broadcast.
    let near_msgs = drain(&mut rx_near);
    for (name, seen) in [
        (
            "FishingCasted",
            near_msgs
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingCasted { .. })),
        ),
        (
            "FishingBite",
            near_msgs
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingBite { .. })),
        ),
        (
            "FishingFight",
            near_msgs
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingFight { .. })),
        ),
        (
            "FishingEnded",
            near_msgs
                .iter()
                .any(|m| matches!(m, ServerMessage::FishingEnded { .. })),
        ),
    ] {
        assert!(seen, "a 13 m bystander must see {name}");
    }
    assert_eq!(
        fishing_msg_count(&drain(&mut rx_far)),
        0,
        "a 63 m player hears nothing"
    );
}

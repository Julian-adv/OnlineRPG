use super::*;

/// Dropping the equipped rod on the ground mid-session is "putting it away"
/// too — the drop path must run the same abort as unequip (this was a real
/// hole: only equip/unequip called the hook).
#[tokio::test(start_paused = true)]
async fn dropping_the_equipped_rod_aborts_the_session() {
    let game_state = make_test_game_state("fishing_drop_rod");
    let (id, mut rx) = make_angler(&game_state, "angler_butterfingers").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
    // 999 is the rod's instance id from make_angler.
    game_state.drop_item(&id, 999).await;

    assert!(
        drain(&mut rx).iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Aborted,
                ..
            }
        )),
        "dropping the rod must abort the session"
    );
    assert!(
        game_state
            .ground_items
            .read()
            .await
            .values()
            .any(|g| g.item.item_def_id == "fishing_rod"),
        "the rod itself lands on the ground"
    );
}

/// A deliberate reel-in mid-struggle ends Aborted and forfeits everything:
/// no consolation XP, no item, no coins — abandoning a hooked fish is the
/// one outcome that pays nothing.
#[tokio::test(start_paused = true)]
async fn stopping_mid_struggle_forfeits_everything() {
    let game_state = make_test_game_state("fishing_stop");
    let (id, mut rx) = make_angler(&game_state, "angler_quitter").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_until_bite(&game_state, &mut rx).await;
    game_state.respond_fishing(&id, FishingAction::Hook).await;
    let _ = drain(&mut rx); // clear the hook confirmation / first round
    game_state.stop_fishing(&id).await;

    let msgs = drain(&mut rx);
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Aborted,
                ..
            }
        )),
        "stop mid-struggle ends Aborted"
    );
    assert!(
        msgs.iter()
            .all(|m| !matches!(m, ServerMessage::SkillXpGained { .. })),
        "abandoning a hooked fish pays no consolation XP"
    );
    assert!(
        msgs.iter()
            .all(|m| !matches!(m, ServerMessage::InventoryUpdated { .. })
                && !matches!(m, ServerMessage::GoldGained { .. })),
        "no item or coins are awarded on a deliberate stop"
    );
}

/// Hooking after the window plus grace has passed is TooLate: the fish
/// escapes with the consolation XP. Locks the three-tier lag contract
/// (in time → hooked; past grace → escaped; the tick reaper only at 2×grace).
#[tokio::test(start_paused = true)]
async fn a_late_hook_escapes_with_consolation_xp() {
    let game_state = make_test_game_state("fishing_late_hook");
    let (id, mut rx) = make_angler(&game_state, "angler_asleep").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_until_bite(&game_state, &mut rx).await;
    // Sleep through the window and the grace — but respond before the tick
    // reaper (which allows 2× grace), so the handler judges the verdict.
    advance(Duration::from_millis(
        u64::from(BITE_WINDOW_MS) + u64::from(LATENCY_GRACE_MS) + 100,
    ))
    .await;
    game_state.respond_fishing(&id, FishingAction::Hook).await;

    let msgs = drain(&mut rx);
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Escaped,
                ..
            }
        )),
        "a hook past the grace window is TooLate → Escaped"
    );
    assert!(
    msgs.iter().any(
        |m| matches!(m, ServerMessage::SkillXpGained { xp_amount, .. } if *xp_amount == ESCAPE_XP)
    ),
    "the late angler still gets the {ESCAPE_XP} XP consolation"
);
}

/// One line per angler: casting again while a session is live is refused
/// and leaves the original session running.
#[tokio::test(start_paused = true)]
async fn casting_twice_is_refused() {
    let game_state = make_test_game_state("fishing_double_cast");
    let (id, mut rx) = make_angler(&game_state, "angler_greedy").await;

    game_state.start_fishing(&id, water_target()).await;
    game_state.start_fishing(&id, water_target()).await;
    assert!(
        drain(&mut rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingError { .. })),
        "the second cast is refused"
    );
    // The original session is intact: its bite still arrives.
    advance_until_bite(&game_state, &mut rx).await;
}

/// The remaining cast validations: a defeated player and a dungeon floor
/// are both refused with a direct error and no session.
#[tokio::test(start_paused = true)]
async fn cast_refused_while_defeated_or_indoors() {
    let game_state = make_test_game_state("fishing_cast_guards");
    let (id, mut rx) = make_angler(&game_state, "angler_unfit").await;

    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 0;
    game_state.start_fishing(&id, water_target()).await;
    assert!(
        drain(&mut rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingError { .. })),
        "a defeated player cannot cast"
    );

    {
        let mut players = game_state.players.write().await;
        let p = players.get_mut(&id).unwrap();
        p.health = 10;
        p.floor_level = 1;
    }
    game_state.start_fishing(&id, water_target()).await;
    assert!(
        drain(&mut rx)
            .iter()
            .any(|m| matches!(m, ServerMessage::FishingError { .. })),
        "no fishing in the dungeon"
    );
    assert!(
        game_state.fishing_sessions.read().await.is_empty(),
        "neither refused cast may leave a session behind"
    );
}

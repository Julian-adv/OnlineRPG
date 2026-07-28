use super::*;

/// The real kill chain: a lethal monster blow must run the death chokepoint
/// and abort the victim's fishing session. This drives `did_die` through
/// `broadcast_monster_attack`, so rewiring the combat path away from
/// `on_player_died` fails here (the direct-call test below only locks the
/// chokepoint's contents, not this wiring — a mutation test proved that).
#[tokio::test(start_paused = true)]
async fn a_lethal_blow_reels_in_the_line() {
    let game_state = make_test_game_state("fishing_combat_death");
    let (id, mut rx) = make_angler(&game_state, "angler_mauled").await;
    let handler = pid("beast_handler");
    game_state
        .add_player(make_player("beast_handler", -99.0, 50.0))
        .await;
    // One hit kills; misses are re-rolled on fresh monsters (each has its
    // own attack cooldown), so the kill is effectively certain.
    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 1;
    {
        let mut monsters = game_state.monsters.write().await;
        for i in 0..20 {
            let mid = format!("killer_{i}");
            let mut monster = make_monster(
                &mid,
                Position {
                    x: -99.0,
                    y: 0.0,
                    z: 50.0,
                },
                0,
            );
            monster.owner_id = Some(handler);
            monsters.insert(mid, monster);
        }
    }

    game_state.start_fishing(&id, water_target()).await;
    advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;

    for i in 0..20 {
        game_state
            .broadcast_monster_attack(&handler, &format!("killer_{i}"), &id)
            .await;
        if game_state.players.read().await.get(&id).unwrap().health == 0 {
            break;
        }
    }
    assert_eq!(
        game_state.players.read().await.get(&id).unwrap().health,
        0,
        "twenty adjacent swings at 1 HP must land a kill"
    );
    assert!(
        drain(&mut rx).iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Aborted,
                ..
            }
        )),
        "a combat death must abort the fishing session through the real kill chain"
    );
}

/// Dying reels the line in: the death chokepoint aborts the session and no
/// bite ever reaches the corpse.
#[tokio::test(start_paused = true)]
async fn death_reels_in_the_line() {
    let game_state = make_test_game_state("fishing_death");
    let (id, mut rx) = make_angler(&game_state, "angler_doomed").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
    game_state.on_player_died(&id).await;

    assert!(
        drain(&mut rx).iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Aborted,
                ..
            }
        )),
        "death must abort the session"
    );
    // Session is gone for real: even the longest wait produces no bite.
    advance_with_ticks(&game_state, u64::from(WAIT_MAX_MS) + 1000).await;
    assert!(
        drain(&mut rx)
            .iter()
            .all(|m| !matches!(m, ServerMessage::FishingBite { .. })),
        "no bite may reach a dead angler"
    );
}

/// Putting the rod away reels the line in.
#[tokio::test(start_paused = true)]
async fn unequipping_the_rod_aborts_the_session() {
    let game_state = make_test_game_state("fishing_unequip");
    let (id, mut rx) = make_angler(&game_state, "angler_fickle").await;

    game_state.start_fishing(&id, water_target()).await;
    advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
    game_state.unequip_item(&id, EquipSlot::MainHand).await;

    assert!(
        drain(&mut rx).iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Aborted,
                ..
            }
        )),
        "stowing the rod must abort the session"
    );
}

/// Swapping a weapon into the main hand displaces the rod — same abort.
#[tokio::test(start_paused = true)]
async fn swapping_a_weapon_into_the_main_hand_aborts_the_session() {
    let game_state = make_test_game_state("fishing_swap");
    let (id, mut rx) = make_angler(&game_state, "angler_armed").await;
    game_state
        .inventories
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .bag
        .push(bag_item(1000, "iron_sword", 1));

    game_state.start_fishing(&id, water_target()).await;
    advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
    game_state.equip_item(&id, 1000).await;

    let main_hand = game_state
        .inventories
        .read()
        .await
        .get(&id)
        .unwrap()
        .equipped
        .get(&EquipSlot::MainHand)
        .unwrap()
        .item_def_id
        .clone();
    assert_eq!(main_hand, "iron_sword", "the sword displaced the rod");
    assert!(
        drain(&mut rx).iter().any(|m| matches!(
            m,
            ServerMessage::FishingEnded {
                outcome: FishingOutcome::Aborted,
                ..
            }
        )),
        "losing the rod to a swap must abort the session"
    );
}

/// Gear changes that leave the rod alone don't break concentration — putting
/// on a cap mid-wait keeps the line wet and the bite still arrives.
#[tokio::test(start_paused = true)]
async fn equipping_armor_does_not_break_concentration() {
    let game_state = make_test_game_state("fishing_armor");
    let (id, mut rx) = make_angler(&game_state, "angler_dapper").await;
    game_state
        .inventories
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .bag
        .push(bag_item(1001, "leather_helmet", 1));

    game_state.start_fishing(&id, water_target()).await;
    advance_with_ticks(&game_state, u64::from(CAST_MS) + 250).await;
    game_state.equip_item(&id, 1001).await;

    assert!(
        drain(&mut rx)
            .iter()
            .all(|m| !matches!(m, ServerMessage::FishingEnded { .. })),
        "a hat must not abort the session"
    );
    // Still fishing: the bite arrives (advance_until_bite panics otherwise).
    advance_until_bite(&game_state, &mut rx).await;
}

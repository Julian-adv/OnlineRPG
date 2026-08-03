use super::*;

#[test]
fn haggling_band_invariant_boundary() {
    // Rica's actual rate must satisfy the invariant; 60% is the first rate
    // where max haggled sell (60% * 1.25) meets min haggled buy (75%).
    assert!(deals::band_invariant_holds(40));
    assert!(deals::band_invariant_holds(59));
    assert!(!deals::band_invariant_holds(60));
}

#[test]
fn haggling_band_widens_with_cha_within_limits() {
    assert_eq!(deals::deal_half_band_pct(10), 10);
    assert_eq!(deals::deal_half_band_pct(3), 5);
    assert_eq!(deals::deal_half_band_pct(13), 16);
    assert_eq!(deals::deal_half_band_pct(18), 25);
    assert_eq!(deals::deal_half_band_pct(255), 25);
}

#[tokio::test]
async fn offer_deal_clamps_modifier_to_cha_band() {
    let game_state = make_test_game_state("offer_clamp");
    let (mut buyer_rx, mut npc_rx) = setup_haggle(&game_state, 10, 0).await;

    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "wooden_shield",
            DealKind::Buy,
            -50,
            "loyal customer",
        )
        .await;

    match buyer_rx.try_recv() {
        Ok(ServerMessage::DealUpdated {
            item_def_id,
            kind,
            modifier_pct,
            ..
        }) => {
            assert_eq!(item_def_id, "wooden_shield");
            assert_eq!(kind, DealKind::Buy);
            assert_eq!(modifier_pct, -10, "CHA 10 band is ±10");
        }
        other => panic!("Expected DealUpdated for buyer, got {:?}", other),
    }
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted,
            applied_modifier_pct,
            ..
        }) => {
            assert!(accepted);
            assert_eq!(applied_modifier_pct, -10);
        }
        other => panic!("Expected DealResult for NPC, got {:?}", other),
    }
}

#[tokio::test]
async fn cross_floor_offer_deal_is_rejected_without_consuming_ledger() {
    let game_state = make_test_game_state("cross_floor_offer");
    let (mut buyer_rx, mut npc_rx) = setup_haggle(&game_state, 18, 0).await;
    set_floor(&game_state, "buyer", 1).await;
    let ledger_before = game_state.deal_ledger_state_for_test("Rica", "buyer").await;

    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "wooden_shield",
            DealKind::Buy,
            -50,
            "loyal customer",
        )
        .await;

    assert!(matches!(buyer_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("another floor"), "got: {message}");
        }
        other => panic!("Expected DealResult rejection for NPC, got {other:?}"),
    }
    assert_eq!(
        game_state.deal_ledger_state_for_test("Rica", "buyer").await,
        ledger_before,
        "rejection must preserve NPC budget, player cap, and cooldown"
    );
    assert!(game_state
        .active_deals_for(&pid("buyer"), "Rica")
        .await
        .is_empty());

    // CHA 18 clamps this to -25%, a 625 discount. The immediate retry proves
    // the rejection did not consume the cooldown; the ledger snapshot above
    // directly proves that it did not consume the player's daily budget.
    set_floor(&game_state, "buyer", 0).await;
    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "wooden_shield",
            DealKind::Buy,
            -50,
            "loyal customer",
        )
        .await;

    match buyer_rx.try_recv() {
        Ok(ServerMessage::DealUpdated { modifier_pct, .. }) => {
            assert_eq!(modifier_pct, -25);
        }
        other => panic!("Expected DealUpdated after same-floor retry, got {other:?}"),
    }
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted,
            applied_modifier_pct,
            ..
        }) => {
            assert!(accepted);
            assert_eq!(applied_modifier_pct, -25);
        }
        other => panic!("Expected accepted DealResult after retry, got {other:?}"),
    }
    let active = game_state.active_deals_for(&pid("buyer"), "Rica").await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].modifier_pct, -25);
}

#[tokio::test]
async fn offer_deal_enforces_cooldown_and_player_budget() {
    let game_state = make_test_game_state("offer_limits");
    let (_buyer_rx, mut npc_rx) = setup_haggle(&game_state, 18, 0).await;

    // First offer: accepted (CHA 18 → band ±25, cost 625 on wooden_shield).
    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "wooden_shield",
            DealKind::Buy,
            -25,
            "first",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult { accepted, .. }) => assert!(accepted),
        other => panic!("Expected accepted DealResult, got {:?}", other),
    }

    // Immediate second offer: rejected by the cooldown.
    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "dagger",
            DealKind::Buy,
            -5,
            "second",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("cooldown"), "got: {message}");
        }
        other => panic!("Expected cooldown rejection, got {:?}", other),
    }

    // Cooldown lifted: five more 625-cost discounts fill the player's
    // daily cap (4000: 6 × 625 = 3750), then the next offer is rejected.
    for _ in 0..5 {
        game_state.clear_deal_cooldowns_for_test().await;
        game_state
            .offer_deal(
                &pid("npc_rica"),
                &pid("buyer"),
                "wooden_shield",
                DealKind::Buy,
                -25,
                "refill",
            )
            .await;
        match npc_rx.try_recv() {
            Ok(ServerMessage::DealResult { accepted, .. }) => assert!(accepted),
            other => panic!("Expected accepted DealResult, got {:?}", other),
        }
    }
    game_state.clear_deal_cooldowns_for_test().await;
    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "wooden_shield",
            DealKind::Buy,
            -25,
            "over cap",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("discount limit"), "got: {message}");
        }
        other => panic!("Expected budget rejection, got {:?}", other),
    }
}

#[tokio::test]
async fn buy_item_applies_deal_once() {
    let game_state = make_test_game_state("buy_with_deal");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 30_000).await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("buyer"), Default::default());
    }

    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "wooden_shield",
            DealKind::Buy,
            -10,
            "deal",
        )
        .await;

    // First buy uses the -10% deal: 2500 → 2250.
    game_state
        .buy_item(&pid("buyer"), &pid("npc_rica"), "wooden_shield")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 27_750);

    // The deal is single-use: the second buy pays full price.
    game_state
        .buy_item(&pid("buyer"), &pid("npc_rica"), "wooden_shield")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 25_250);
}

#[tokio::test]
async fn sell_item_applies_deal_bonus() {
    let game_state = make_test_game_state("sell_with_deal");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 18, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(onlinerpg_shared::inventory::ItemInstance {
            instance_id: 7,
            item_def_id: "iron_sword".to_string(),
            quantity: 1,
            enchant: 0,
            durability: None,
        });
        inventories.insert(pid("buyer"), inv);
    }

    game_state
        .offer_deal(
            &pid("npc_rica"),
            &pid("buyer"),
            "iron_sword",
            DealKind::Sell,
            25,
            "today's wanted item",
        )
        .await;

    // Sell rate 40% with a +25% bonus: 10000 * 0.4 * 1.25 = 5000.
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 5_000);
}

#[tokio::test]
async fn cross_floor_shop_actions_leave_economy_state_unchanged() {
    let game_state = make_test_game_state("cross_floor_shop");
    let (mut buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    game_state.inventories.write().await.insert(
        pid("buyer"),
        PlayerInventory {
            bag: vec![bag_item(7, "iron_sword", 1), bag_item(8, "dagger", 1)],
            ..Default::default()
        },
    );

    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };
    set_floor(&game_state, "buyer", 1).await;

    while buyer_rx.try_recv().is_ok() {}
    let gold_before = game_state.get_player_gold(&pid("buyer")).await;
    let bag_before = game_state.inventories.read().await[&pid("buyer")]
        .bag
        .clone();

    game_state
        .open_shop(&pid("buyer"), &pid("npc_rica"), true)
        .await;
    game_state
        .buy_item(&pid("buyer"), &pid("npc_rica"), "iron_sword")
        .await;
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 8)
        .await;
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;

    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, gold_before);
    let bag_after = game_state.inventories.read().await[&pid("buyer")]
        .bag
        .clone();
    assert_eq!(bag_after, bag_before);
    assert!(
        game_state.open_shops.read().await.is_empty(),
        "a rejected shop open must not register a hold"
    );
    assert_eq!(
        game_state.buybacks.read().await[&(1, "Rica".to_string())].len(),
        1,
        "a rejected buyback must remain available"
    );
    for _ in 0..4 {
        match buyer_rx.try_recv() {
            Ok(ServerMessage::TradeError { message }) => {
                assert!(message.contains("another floor"), "got: {message}")
            }
            other => panic!("Expected cross-floor TradeError, got {other:?}"),
        }
    }
}

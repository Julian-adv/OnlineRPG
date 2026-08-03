use super::*;

// --- Resident (non-merchant) trading (economy phase 3) ---

/// Spawn the resident trader Karl (wishlist: torch, dagger @120%) next to a
/// seller. Karl's wallet and bag are set explicitly; the seller starts with
/// the given bag and no gold.
async fn setup_resident_trade(
    game_state: &GameState,
    npc_gold: i64,
    npc_bag: Vec<ItemInstance>,
    seller_bag: Vec<ItemInstance>,
) {
    game_state
        .add_player(make_npc("npc_karl", "Karl", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("seller", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("seller"), 1, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state
        .register_player_character(&pid("npc_karl"), 2, 0, attrs_with_cha(10), npc_gold, None)
        .await;
    let mut inventories = game_state.inventories.write().await;
    inventories.insert(
        pid("npc_karl"),
        onlinerpg_shared::inventory::PlayerInventory {
            bag: npc_bag,
            ..Default::default()
        },
    );
    inventories.insert(
        pid("seller"),
        onlinerpg_shared::inventory::PlayerInventory {
            bag: seller_bag,
            ..Default::default()
        },
    );
}

#[tokio::test]
async fn resident_buys_wishlist_item_at_premium_from_wallet() {
    let game_state = make_test_game_state("resident_sell");
    setup_resident_trade(&game_state, 10_000, vec![], vec![bag_item(7, "torch", 1)]).await;

    // Torch base 50 at Karl's 120% wishlist rate → 60.
    game_state
        .sell_item(&pid("seller"), &pid("npc_karl"), 7)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 60);
    assert_eq!(
        game_state.get_player_gold(&pid("npc_karl")).await,
        10_000 - 60
    );

    // The torch landed in Karl's real inventory; the seller's bag is empty.
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("npc_karl")].bag.len(), 1);
    assert_eq!(inventories[&pid("npc_karl")].bag[0].item_def_id, "torch");
    assert!(inventories[&pid("seller")].bag.is_empty());
}

#[tokio::test]
async fn resident_rejects_items_off_the_wishlist() {
    let game_state = make_test_game_state("resident_off_wishlist");
    setup_resident_trade(
        &game_state,
        10_000,
        vec![],
        vec![bag_item(7, "iron_sword", 1)],
    )
    .await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    game_state
        .sell_item(&pid("seller"), &pid("npc_karl"), 7)
        .await;

    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 0);
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("no use"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
    let inventories = game_state.inventories.read().await;
    assert_eq!(
        inventories[&pid("seller")].bag.len(),
        1,
        "item must be retained"
    );
}

#[tokio::test]
async fn resident_wallet_caps_purchases() {
    let game_state = make_test_game_state("resident_wallet_cap");
    // Karl has 59 gold units; the torch costs him 60.
    setup_resident_trade(&game_state, 59, vec![], vec![bag_item(7, "torch", 1)]).await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    game_state
        .sell_item(&pid("seller"), &pid("npc_karl"), 7)
        .await;

    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 0);
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 59);
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("afford"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
}

#[tokio::test]
async fn resident_sells_stock_but_keeps_wishlist_items() {
    let game_state = make_test_game_state("resident_stock");
    // Karl carries a spear (sellable stock) and a torch (wishlist: kept).
    setup_resident_trade(
        &game_state,
        0,
        vec![bag_item(11, "spear", 1), bag_item(12, "torch", 1)],
        vec![],
    )
    .await;
    {
        let mut gold = game_state.player_gold.write().await;
        gold.insert(pid("seller"), 10_000);
    }
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    // Spear base 3500 — instance moves to the buyer, gold to Karl.
    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "spear")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 6_500);
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 3_500);
    {
        let inventories = game_state.inventories.read().await;
        assert_eq!(inventories[&pid("seller")].bag.len(), 1);
        assert_eq!(inventories[&pid("seller")].bag[0].item_def_id, "spear");
        assert_eq!(inventories[&pid("npc_karl")].bag.len(), 1);
        assert_eq!(inventories[&pid("npc_karl")].bag[0].item_def_id, "torch");
    }
    while seller_rx.try_recv().is_ok() {}

    // The torch is on Karl's wishlist: he keeps it (no buy-back pump).
    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "torch")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 6_500);
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("part with"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
}

#[tokio::test]
async fn resident_sale_preserves_damaged_armor_condition() {
    let game_state = make_test_game_state("resident_durability_transfer");
    let mut armor = bag_item(11, "leather_armor", 1);
    armor.durability = Some(23);
    setup_resident_trade(&game_state, 0, vec![armor], vec![]).await;
    game_state
        .player_gold
        .write()
        .await
        .insert(pid("seller"), 10_000);

    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "leather_armor")
        .await;

    let inventories = game_state.inventories.read().await;
    assert!(inventories[&pid("npc_karl")].bag.is_empty());
    assert_eq!(inventories[&pid("seller")].bag[0].durability, Some(23));
}

#[tokio::test]
async fn resident_shop_state_reports_wishlist_and_stock() {
    let game_state = make_test_game_state("resident_shop_state");
    setup_resident_trade(
        &game_state,
        4_321,
        vec![
            bag_item(11, "spear", 1),
            bag_item(12, "torch", 1),
            bag_item(13, "worn_iron_sword", 1),
        ],
        vec![],
    )
    .await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    game_state
        .open_shop(&pid("seller"), &pid("npc_karl"), true)
        .await;

    match seller_rx.try_recv() {
        Ok(ServerMessage::ShopState {
            merchant_name,
            catalog,
            sell_rate_percent,
            wishlist,
            stock,
            ..
        }) => {
            assert_eq!(merchant_name, "Karl");
            assert!(catalog.is_empty());
            assert_eq!(sell_rate_percent, 120);
            assert_eq!(wishlist, vec!["torch".to_string(), "dagger".to_string()]);
            // Stock excludes the wishlist torch and the unpriced worn sword.
            assert_eq!(stock.len(), 1);
            assert_eq!(stock[0].item_def_id, "spear");
            assert_eq!(stock[0].quantity, 1);
        }
        other => panic!("Expected ShopState, got {:?}", other),
    }
}

#[tokio::test]
async fn resident_deal_band_is_wider_and_wishlist_scoped() {
    let game_state = make_test_game_state("resident_deal_band");
    setup_resident_trade(&game_state, 10_000, vec![], vec![]).await;
    let mut npc_rx = game_state.register_direct_channel(&pid("npc_karl")).await;

    // CHA 10 resident band is ±20 (twice the merchant ±10).
    game_state
        .offer_deal(
            &pid("npc_karl"),
            &pid("seller"),
            "torch",
            DealKind::Sell,
            40,
            "really need torches tonight",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted,
            applied_modifier_pct,
            ..
        }) => {
            assert!(accepted);
            assert_eq!(applied_modifier_pct, 20);
        }
        other => panic!("Expected DealResult, got {:?}", other),
    }

    // Sell offers outside the wishlist are rejected.
    game_state.clear_deal_cooldowns_for_test().await;
    game_state
        .offer_deal(
            &pid("npc_karl"),
            &pid("seller"),
            "iron_sword",
            DealKind::Sell,
            10,
            "nice sword",
        )
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::DealResult {
            accepted, message, ..
        }) => {
            assert!(!accepted);
            assert!(message.contains("wishlist"), "got: {message}");
        }
        other => panic!("Expected rejection, got {:?}", other),
    }
}

#[tokio::test]
async fn open_trade_pushes_shop_state_to_the_player() {
    let game_state = make_test_game_state("open_trade");
    setup_resident_trade(&game_state, 1_000, vec![], vec![]).await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;
    let npc_rx = game_state.register_direct_channel(&pid("npc_karl")).await;

    game_state
        .open_trade(&pid("npc_karl"), &pid("seller"))
        .await;
    match seller_rx.try_recv() {
        Ok(ServerMessage::ShopState { merchant_name, .. }) => assert_eq!(merchant_name, "Karl"),
        other => panic!("Expected ShopState, got {:?}", other),
    }

    // A non-trading NPC cannot push a window; the seller hears nothing.
    game_state
        .add_player({
            let mut p = make_player("npc_nobody", 0.5, 0.0);
            p.name = "Nobody".to_string();
            p.is_official_npc = true;
            p
        })
        .await;
    let mut nobody_rx = game_state.register_direct_channel(&pid("npc_nobody")).await;
    game_state
        .open_trade(&pid("npc_nobody"), &pid("seller"))
        .await;
    match nobody_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("nothing to trade"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
    drop(npc_rx);
}

#[tokio::test]
async fn cross_floor_open_trade_is_rejected_before_reaching_the_player() {
    let game_state = make_test_game_state("cross_floor_open_trade");
    setup_resident_trade(&game_state, 1_000, vec![], vec![]).await;
    set_floor(&game_state, "seller", 1).await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;
    let mut npc_rx = game_state.register_direct_channel(&pid("npc_karl")).await;

    game_state
        .open_trade(&pid("npc_karl"), &pid("seller"))
        .await;

    assert!(matches!(seller_rx.try_recv(), Err(MpscTryRecvError::Empty)));
    match npc_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("another floor"), "got: {message}")
        }
        other => panic!("Expected cross-floor TradeError, got {other:?}"),
    }
}

#[tokio::test]
async fn salary_pays_once_per_day_rollover_up_to_cap() {
    let game_state = make_test_game_state("salary");
    setup_resident_trade(&game_state, 27_000, vec![], vec![]).await;

    // First tick after boot only records the day.
    game_state.tick_npc_salaries().await;
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 27_000);

    // Roll the ledger back a day: the next tick pays one salary, capped at
    // the 30_000 wallet cap (27_000 + 5_000 → 30_000).
    {
        let mut last = game_state.npc_salary_last_day.write().await;
        *last = last.map(|d| d - 1);
    }
    game_state.tick_npc_salaries().await;
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 30_000);

    // Same day again: no double payment.
    game_state.tick_npc_salaries().await;
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 30_000);
}

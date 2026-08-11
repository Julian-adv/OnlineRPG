use super::*;
use onlinerpg_shared::messages::{BagLineItem, TradeLineItem};

// --- Resident (non-merchant) trading (economy phase 3) ---

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
    // Karl carries a shield (sellable stock) and a torch (wishlist: kept).
    setup_resident_trade(
        &game_state,
        0,
        vec![bag_item(11, "wooden_shield", 1), bag_item(12, "torch", 1)],
        vec![],
    )
    .await;
    {
        let mut gold = game_state.player_gold.write().await;
        gold.insert(pid("seller"), 10_000);
    }
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    // Shield base 2500 — instance moves to the buyer, gold to Karl.
    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "wooden_shield")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 7_500);
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 2_500);
    {
        let inventories = game_state.inventories.read().await;
        assert_eq!(inventories[&pid("seller")].bag.len(), 1);
        assert_eq!(
            inventories[&pid("seller")].bag[0].item_def_id,
            "wooden_shield"
        );
        assert_eq!(inventories[&pid("npc_karl")].bag.len(), 1);
        assert_eq!(inventories[&pid("npc_karl")].bag[0].item_def_id, "torch");
    }
    while seller_rx.try_recv().is_ok() {}

    // The torch is on Karl's wishlist: he keeps it (no buy-back pump).
    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "torch")
        .await;
    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 7_500);
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("part with"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
}

#[tokio::test]
async fn resident_sale_preserves_enchantment() {
    let game_state = make_test_game_state("resident_enchanted_stock");
    setup_resident_trade(
        &game_state,
        0,
        vec![enchanted_bag_item(11, "wooden_shield", 1, 3)],
        vec![],
    )
    .await;
    game_state
        .player_gold
        .write()
        .await
        .insert(pid("seller"), 10_000);

    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "wooden_shield")
        .await;

    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("seller")].bag[0].enchant, 3);
    assert!(inventories[&pid("npc_karl")].bag.is_empty());
}

#[tokio::test]
async fn resident_sells_lowest_enchantment_first() {
    let game_state = make_test_game_state("resident_mixed_enchanted_stock");
    setup_resident_trade(
        &game_state,
        0,
        vec![
            enchanted_bag_item(11, "wooden_shield", 1, 3),
            bag_item(12, "wooden_shield", 1),
        ],
        vec![],
    )
    .await;
    game_state
        .player_gold
        .write()
        .await
        .insert(pid("seller"), 10_000);

    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "wooden_shield")
        .await;

    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("seller")].bag[0].enchant, 0);
    assert_eq!(inventories[&pid("npc_karl")].bag[0].enchant, 3);
}

#[tokio::test]
async fn resident_sale_takes_one_unit_out_of_a_stack() {
    let game_state = make_test_game_state("resident_partial_stack");
    setup_resident_trade(
        &game_state,
        0,
        vec![bag_item(11, "healing_potion", 5)],
        vec![],
    )
    .await;
    game_state
        .player_gold
        .write()
        .await
        .insert(pid("seller"), 10_000);

    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "healing_potion")
        .await;

    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 9_400);
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 600);
    let inventories = game_state.inventories.read().await;
    let buyer_bag = &inventories[&pid("seller")].bag;
    assert_eq!(buyer_bag.len(), 1);
    assert_eq!(buyer_bag[0].item_def_id, "healing_potion");
    assert_eq!(buyer_bag[0].quantity, 1);
    let npc_bag = &inventories[&pid("npc_karl")].bag;
    assert_eq!(npc_bag.len(), 1, "the rest of the stack stays in stock");
    assert_eq!(npc_bag[0].quantity, 4);
}

#[tokio::test]
async fn resident_shop_state_reports_wishlist_and_stock() {
    let game_state = make_test_game_state("resident_shop_state");
    setup_resident_trade(
        &game_state,
        4_321,
        vec![
            bag_item(11, "wooden_shield", 1),
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
            assert_eq!(stock[0].item_def_id, "wooden_shield");
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

/// A player's "Not now" on a pushed trade window reaches the NPC as
/// TradeDeclined, so its agent can let trading rest. Declines from an NPC
/// or aimed at a non-NPC are dropped, not relayed.
#[tokio::test]
async fn a_waved_off_trade_window_reaches_the_npc() {
    let game_state = make_test_game_state("decline_trade");
    setup_resident_trade(&game_state, 1_000, vec![], vec![]).await;
    let mut npc_rx = game_state.register_direct_channel(&pid("npc_karl")).await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    game_state
        .decline_trade(&pid("seller"), &pid("npc_karl"))
        .await;
    match npc_rx.try_recv() {
        Ok(ServerMessage::TradeDeclined {
            player_id,
            player_name,
        }) => {
            assert_eq!(player_id, pid("seller"));
            assert_eq!(player_name, "seller");
        }
        other => panic!("Expected TradeDeclined, got {:?}", other),
    }

    game_state
        .decline_trade(&pid("npc_karl"), &pid("seller"))
        .await;
    assert!(
        matches!(seller_rx.try_recv(), Err(MpscTryRecvError::Empty)),
        "a decline aimed at a non-NPC must not be relayed"
    );
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

// --- Keepsakes: offer-only resident stock (Signe's mandolin) ---

#[tokio::test]
async fn keepsake_is_hidden_from_walk_in_stock_until_offered() {
    let game_state = make_test_game_state("keepsake_stock");
    setup_keepsake_trade(
        &game_state,
        vec![bag_item(11, "mandolin", 1), bag_item(12, "spear", 1)],
        10_000,
    )
    .await;
    let mut buyer_rx = game_state.register_direct_channel(&pid("buyer")).await;

    game_state
        .open_shop(&pid("buyer"), &pid("npc_signe"), true)
        .await;
    match buyer_rx.try_recv() {
        Ok(ServerMessage::ShopState { stock, .. }) => {
            assert_eq!(stock.len(), 1, "keepsake must not show to walk-ins");
            assert_eq!(stock[0].item_def_id, "spear");
        }
        other => panic!("Expected ShopState, got {:?}", other),
    }

    game_state
        .offer_deal(
            &pid("npc_signe"),
            &pid("buyer"),
            "mandolin",
            DealKind::Buy,
            -10,
            "a friend of the music",
        )
        .await;
    while buyer_rx.try_recv().is_ok() {}

    game_state
        .open_shop(&pid("buyer"), &pid("npc_signe"), true)
        .await;
    match buyer_rx.try_recv() {
        Ok(ServerMessage::ShopState { stock, .. }) => {
            assert!(
                stock.iter().any(|s| s.item_def_id == "mandolin"),
                "the offered keepsake appears for the offeree"
            );
        }
        other => panic!("Expected ShopState, got {:?}", other),
    }
}

#[tokio::test]
async fn keepsake_sells_only_through_a_personal_offer() {
    let game_state = make_test_game_state("keepsake_buy");
    setup_keepsake_trade(&game_state, vec![bag_item(11, "mandolin", 1)], 10_000).await;
    let mut buyer_rx = game_state.register_direct_channel(&pid("buyer")).await;

    game_state
        .buy_item(&pid("buyer"), &pid("npc_signe"), "mandolin")
        .await;
    match buyer_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("part with"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 10_000);

    // Mandolin base 4000 at -10% → 3600 (CHA 10 resident band is ±20).
    game_state
        .offer_deal(
            &pid("npc_signe"),
            &pid("buyer"),
            "mandolin",
            DealKind::Buy,
            -10,
            "she has earned it",
        )
        .await;
    game_state
        .buy_item(&pid("buyer"), &pid("npc_signe"), "mandolin")
        .await;

    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 6_400);
    assert_eq!(game_state.get_player_gold(&pid("npc_signe")).await, 3_600);
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("buyer")].bag[0].item_def_id, "mandolin");
    assert!(inventories[&pid("npc_signe")].bag.is_empty());
}

#[tokio::test]
async fn keepsake_batch_buy_is_one_unit_behind_the_offer() {
    let game_state = make_test_game_state("keepsake_batch");
    setup_keepsake_trade(&game_state, vec![bag_item(11, "mandolin", 1)], 10_000).await;
    let mut buyer_rx = game_state.register_direct_channel(&pid("buyer")).await;

    game_state
        .buy_items(
            &pid("buyer"),
            &pid("npc_signe"),
            vec![TradeLineItem {
                item_def_id: "mandolin".to_string(),
                qty: 2,
            }],
        )
        .await;
    match buyer_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("only part with one"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }

    game_state
        .buy_items(
            &pid("buyer"),
            &pid("npc_signe"),
            vec![TradeLineItem {
                item_def_id: "mandolin".to_string(),
                qty: 1,
            }],
        )
        .await;
    match buyer_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("part with"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }

    game_state
        .offer_deal(
            &pid("npc_signe"),
            &pid("buyer"),
            "mandolin",
            DealKind::Buy,
            0,
            "a regular",
        )
        .await;
    game_state
        .buy_items(
            &pid("buyer"),
            &pid("npc_signe"),
            vec![TradeLineItem {
                item_def_id: "mandolin".to_string(),
                qty: 1,
            }],
        )
        .await;

    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 6_000);
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("buyer")].bag[0].item_def_id, "mandolin");
    assert!(inventories[&pid("npc_signe")].bag.is_empty());
}

#[tokio::test]
async fn seed_npc_keepsakes_refills_missing_items_once() {
    let game_state = make_test_game_state("keepsake_seed");
    setup_keepsake_trade(&game_state, vec![], 0).await;

    game_state
        .seed_npc_keepsakes(&pid("npc_signe"), "Signe")
        .await;
    game_state
        .seed_npc_keepsakes(&pid("npc_signe"), "Signe")
        .await;
    {
        let inventories = game_state.inventories.read().await;
        let bag = &inventories[&pid("npc_signe")].bag;
        assert_eq!(bag.len(), 2, "seeding twice must not duplicate");
        assert!(bag.iter().any(|i| i.item_def_id == "mandolin"));
        assert!(bag.iter().any(|i| i.item_def_id == "worn_mandolin"));
    }

    // An equipped keepsake still counts as owned.
    {
        let mut inventories = game_state.inventories.write().await;
        let inv = inventories.get_mut(&pid("npc_signe")).unwrap();
        let pos = inv
            .bag
            .iter()
            .position(|i| i.item_def_id == "worn_mandolin")
            .unwrap();
        let item = inv.bag.remove(pos);
        inv.equipped.insert(EquipSlot::MainHand, item);
    }
    game_state
        .seed_npc_keepsakes(&pid("npc_signe"), "Signe")
        .await;
    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&pid("npc_signe")].bag;
    assert_eq!(bag.len(), 1);
    assert_eq!(bag[0].item_def_id, "mandolin");
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

// --- Loadout: issued gear seeded on join, never sold ---

#[tokio::test]
async fn a_registry_loadout_is_granted_and_worn_on_join() {
    let game_state = make_test_game_state("loadout_seed");
    setup_resident_trade(&game_state, 0, vec![], vec![]).await;
    let def = crate::npc_defs::npc_defs().get_by_npc_name("Karl").unwrap();
    let worn = def
        .loadout
        .iter()
        .filter_map(|id| game_state.item_defs.get(id).and_then(|d| d.equip_slot))
        .collect::<std::collections::HashSet<_>>()
        .len();

    game_state.seed_npc_loadout(&pid("npc_karl"), "Karl").await;
    {
        let inventories = game_state.inventories.read().await;
        let inv = &inventories[&pid("npc_karl")];
        for id in &def.loadout {
            assert!(inv.has_item(id), "missing loadout item {id}");
        }
        assert_eq!(inv.equipped.len(), worn, "distinct-slot items are worn");
    }

    // A later join grants nothing on top.
    game_state.seed_npc_loadout(&pid("npc_karl"), "Karl").await;
    let inventories = game_state.inventories.read().await;
    let inv = &inventories[&pid("npc_karl")];
    assert_eq!(inv.items().count(), def.loadout.len());
}

#[tokio::test]
async fn loadout_gear_is_not_for_sale_even_from_the_bag() {
    let game_state = make_test_game_state("loadout_stock");
    setup_resident_trade(
        &game_state,
        0,
        vec![bag_item(11, "spear", 1), bag_item(12, "iron_sword", 1)],
        vec![],
    )
    .await;
    game_state
        .register_player_character(&pid("seller"), 1, 0, attrs_with_cha(10), 50_000, None)
        .await;
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    game_state
        .open_shop(&pid("seller"), &pid("npc_karl"), true)
        .await;
    match seller_rx.try_recv() {
        Ok(ServerMessage::ShopState { stock, .. }) => {
            assert_eq!(stock.len(), 1, "issued gear must not show in stock");
            assert_eq!(stock[0].item_def_id, "iron_sword");
        }
        other => panic!("Expected ShopState, got {:?}", other),
    }
    let _gold_update = seller_rx.try_recv();

    game_state
        .buy_item(&pid("seller"), &pid("npc_karl"), "spear")
        .await;
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("part with"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
}

#[tokio::test]
async fn an_npc_never_sells_its_issued_gear_to_a_merchant() {
    let game_state = make_test_game_state("npc_sells_loadout");
    game_state
        .add_player(make_npc("npc_rica", "Rica", 0.0, 0.0))
        .await;
    game_state
        .add_player(make_npc("npc_karl", "Karl", 1.0, 0.0))
        .await;
    game_state
        .register_player_character(&pid("npc_karl"), 2, 0, attrs_with_cha(10), 0, None)
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(
            pid("npc_karl"),
            PlayerInventory {
                bag: vec![bag_item(11, "spear", 1)],
                ..Default::default()
            },
        );
    }
    let mut karl_rx = game_state.register_direct_channel(&pid("npc_karl")).await;

    game_state
        .sell_item(&pid("npc_karl"), &pid("npc_rica"), 11)
        .await;
    match karl_rx.try_recv() {
        Ok(ServerMessage::TradeError { message }) => {
            assert!(message.contains("issued gear"), "got: {message}")
        }
        other => panic!("Expected TradeError, got {:?}", other),
    }
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("npc_karl")].bag.len(), 1, "spear retained");
}

#[tokio::test]
async fn an_npc_never_drops_its_issued_gear() {
    let game_state = make_test_game_state("npc_drops_loadout");
    game_state
        .add_player(make_npc("npc_karl", "Karl", 0.0, 0.0))
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv = PlayerInventory {
            bag: vec![bag_item(11, "spear", 1), bag_item(12, "torch", 1)],
            ..Default::default()
        };
        inv.equipped
            .insert(EquipSlot::Chest, bag_item(13, "leather_armor", 1));
        inventories.insert(pid("npc_karl"), inv);
    }
    let mut karl_rx = game_state.register_direct_channel(&pid("npc_karl")).await;

    // Bagged and worn loadout gear alike stay put.
    for instance_id in [11, 13] {
        game_state.drop_item(&pid("npc_karl"), instance_id).await;
        match karl_rx.try_recv() {
            Ok(ServerMessage::SystemMessage { message }) => {
                assert!(message.contains("issued gear"), "got: {message}")
            }
            other => panic!("Expected SystemMessage, got {:?}", other),
        }
    }

    // One issued line poisons the whole batch — the torch stays too.
    game_state
        .drop_items(
            &pid("npc_karl"),
            vec![
                BagLineItem {
                    instance_id: 12,
                    qty: 1,
                },
                BagLineItem {
                    instance_id: 11,
                    qty: 1,
                },
            ],
        )
        .await;
    match karl_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("issued gear"), "got: {message}")
        }
        other => panic!("Expected SystemMessage, got {:?}", other),
    }

    let inventories = game_state.inventories.read().await;
    let inv = &inventories[&pid("npc_karl")];
    assert_eq!(inv.bag.len(), 2, "bagged items retained");
    assert_eq!(inv.equipped.len(), 1, "worn gear retained");
    assert!(game_state.ground_items.read().await.is_empty());
}

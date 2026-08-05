use super::*;
use onlinerpg_shared::messages::{BagLineItem, TradeLineItem};

// --- Batched sell/buy/drop (bag-cleanup UX: quantity popups + one
// all-or-nothing round trip instead of N single-unit calls) ---

#[tokio::test]
async fn sell_items_batch_sells_partial_quantity_and_records_one_buyback_per_unit() {
    let game_state = make_test_game_state("batch_sell_partial");
    game_state
        .add_player(make_npc("npc_rica", "Rica", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("seller", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("seller"), 1, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state.inventories.write().await.insert(
        pid("seller"),
        PlayerInventory {
            bag: vec![bag_item(7, "healing_potion", 5)],
            ..Default::default()
        },
    );

    // Rica: 40% sell rate, healing_potion base 600 -> 240/unit.
    game_state
        .sell_items(
            &pid("seller"),
            &pid("npc_rica"),
            vec![BagLineItem {
                instance_id: 7,
                qty: 3,
            }],
        )
        .await;

    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 720);
    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&pid("seller")].bag;
    assert_eq!(bag.len(), 1, "the remaining 2 units stay as one stack");
    assert_eq!(bag[0].quantity, 2);
}

#[tokio::test]
async fn sell_and_buyback_batches_preserve_damaged_armor_condition() {
    let game_state = make_test_game_state("batch_armor_condition_roundtrip");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    let mut armor = bag_item(7, "leather_armor", 1);
    armor.durability = Some(17);
    game_state.inventories.write().await.insert(
        pid("buyer"),
        PlayerInventory {
            bag: vec![armor],
            ..Default::default()
        },
    );

    game_state
        .sell_items(
            &pid("buyer"),
            &pid("npc_rica"),
            vec![BagLineItem {
                instance_id: 7,
                qty: 1,
            }],
        )
        .await;

    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 1_104);
    let entry = game_state.buybacks.read().await[&(1, "Rica".to_string())][0]
        .entry
        .clone();
    assert_eq!(entry.price, 1_104);
    assert_eq!(entry.durability, Some(17));

    game_state
        .buyback_items(&pid("buyer"), &pid("npc_rica"), vec![entry.entry_id])
        .await;

    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 0);
    let inventories = game_state.inventories.read().await;
    let restored = &inventories[&pid("buyer")].bag[0];
    assert_eq!(restored.item_def_id, "leather_armor");
    assert_eq!(restored.durability, Some(17));
}

#[tokio::test]
async fn sell_items_batch_is_all_or_nothing_when_a_line_is_invalid() {
    let game_state = make_test_game_state("batch_sell_atomic");
    game_state
        .add_player(make_npc("npc_rica", "Rica", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("seller", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("seller"), 1, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state.inventories.write().await.insert(
        pid("seller"),
        PlayerInventory {
            bag: vec![bag_item(7, "healing_potion", 5)],
            ..Default::default()
        },
    );
    let mut seller_rx = game_state.register_direct_channel(&pid("seller")).await;

    // Second line references an instance_id that doesn't exist in the bag.
    game_state
        .sell_items(
            &pid("seller"),
            &pid("npc_rica"),
            vec![
                BagLineItem {
                    instance_id: 7,
                    qty: 3,
                },
                BagLineItem {
                    instance_id: 999,
                    qty: 1,
                },
            ],
        )
        .await;

    assert_eq!(
        game_state.get_player_gold(&pid("seller")).await,
        0,
        "no partial payout"
    );
    let inventories = game_state.inventories.read().await;
    assert_eq!(
        inventories[&pid("seller")].bag[0].quantity,
        5,
        "the valid line must not apply either"
    );
    match seller_rx.try_recv() {
        Ok(ServerMessage::TradeError { .. }) => {}
        other => panic!("Expected TradeError, got {:?}", other),
    }
}

#[tokio::test]
async fn sell_items_batch_resident_is_all_or_nothing_on_insufficient_wallet() {
    let game_state = make_test_game_state("batch_sell_resident_atomic");
    // Torches aren't stackable, so two purchased separately stay as two
    // instances; Karl's wishlist pays 60 each (base 50 @ 120%) — 120 total,
    // more than his 100-copper wallet.
    setup_resident_trade(
        &game_state,
        100,
        vec![],
        vec![bag_item(1, "torch", 1), bag_item(2, "torch", 1)],
    )
    .await;

    game_state
        .sell_items(
            &pid("seller"),
            &pid("npc_karl"),
            vec![
                BagLineItem {
                    instance_id: 1,
                    qty: 1,
                },
                BagLineItem {
                    instance_id: 2,
                    qty: 1,
                },
            ],
        )
        .await;

    assert_eq!(game_state.get_player_gold(&pid("seller")).await, 0);
    assert_eq!(game_state.get_player_gold(&pid("npc_karl")).await, 100);
    let inventories = game_state.inventories.read().await;
    assert_eq!(
        inventories[&pid("seller")].bag.len(),
        2,
        "both torches kept"
    );
    assert!(inventories[&pid("npc_karl")].bag.is_empty());
}

#[tokio::test]
async fn buy_items_batch_merges_stackables_and_splits_non_stackables() {
    let game_state = make_test_game_state("batch_buy");
    game_state
        .add_player(make_npc("npc_rica", "Rica", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("buyer", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("buyer"), 1, 0, attrs_with_cha(10), 10_000, None)
        .await;
    game_state.inventories.write().await.insert(
        pid("buyer"),
        PlayerInventory {
            bag: vec![bag_item(5, "healing_potion", 2)],
            ..Default::default()
        },
    );

    game_state
        .buy_items(
            &pid("buyer"),
            &pid("npc_rica"),
            vec![
                TradeLineItem {
                    item_def_id: "healing_potion".to_string(),
                    qty: 3,
                },
                TradeLineItem {
                    item_def_id: "torch".to_string(),
                    qty: 2,
                },
            ],
        )
        .await;

    // healing_potion (600 * 3) + torch (50 * 2) = 1900.
    assert_eq!(
        game_state.get_player_gold(&pid("buyer")).await,
        10_000 - 1_900
    );
    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&pid("buyer")].bag;
    let potion = bag
        .iter()
        .find(|i| i.item_def_id == "healing_potion")
        .expect("existing stack kept");
    assert_eq!(
        potion.instance_id, 5,
        "merged into the existing stack, not a new one"
    );
    assert_eq!(potion.quantity, 5);
    let torches: Vec<_> = bag.iter().filter(|i| i.item_def_id == "torch").collect();
    assert_eq!(
        torches.len(),
        2,
        "non-stackable units stay as separate instances"
    );
    assert!(torches.iter().all(|t| t.quantity == 1));
}

/// The resident's receipt draws from a range reserved before the locks, so
/// every unit must still land on its own id. (No wishlist item is stackable,
/// so the merge branch is unreachable from data alone.)
#[tokio::test]
async fn sell_items_batch_gives_each_resident_unit_its_own_id() {
    let game_state = make_test_game_state("batch_sell_resident_stacks");
    setup_resident_trade(
        &game_state,
        10_000,
        vec![bag_item(50, "healing_potion", 1)],
        vec![bag_item(1, "torch", 1), bag_item(2, "torch", 1)],
    )
    .await;

    game_state
        .sell_items(
            &pid("seller"),
            &pid("npc_karl"),
            vec![
                BagLineItem {
                    instance_id: 1,
                    qty: 1,
                },
                BagLineItem {
                    instance_id: 2,
                    qty: 1,
                },
            ],
        )
        .await;

    let inventories = game_state.inventories.read().await;
    let karl = &inventories[&pid("npc_karl")].bag;
    assert_eq!(
        karl.iter().filter(|i| i.item_def_id == "torch").count(),
        2,
        "non-stackables keep one slot per unit"
    );
    let ids: Vec<_> = karl.iter().map(|i| i.instance_id).collect();
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        ids.len(),
        "every received unit gets its own id"
    );
}

#[tokio::test]
async fn buyback_items_batch_rejoins_one_stack() {
    let game_state = make_test_game_state("batch_buyback_stacks");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 10_000).await;
    game_state.inventories.write().await.insert(
        pid("buyer"),
        PlayerInventory {
            bag: vec![bag_item(7, "healing_potion", 3)],
            ..Default::default()
        },
    );

    game_state
        .sell_items(
            &pid("buyer"),
            &pid("npc_rica"),
            vec![BagLineItem {
                instance_id: 7,
                qty: 2,
            }],
        )
        .await;
    let entry_ids: Vec<u64> = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())]
            .iter()
            .map(|s| s.entry.entry_id)
            .collect()
    };
    assert_eq!(entry_ids.len(), 2, "one buyback entry per sold unit");

    game_state
        .buyback_items(&pid("buyer"), &pid("npc_rica"), entry_ids)
        .await;

    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&pid("buyer")].bag;
    assert_eq!(bag.len(), 1, "both repurchased units rejoin the one stack");
    assert_eq!(bag[0].instance_id, 7);
    assert_eq!(bag[0].quantity, 3);
}

/// An aborted batch must put back every deal it took — otherwise a failed
/// trade silently burns the player's haggled discounts.
#[tokio::test]
async fn buy_items_batch_restores_every_deal_when_it_aborts() {
    let game_state = make_test_game_state("batch_buy_deal_restore");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 100).await;
    game_state
        .inventories
        .write()
        .await
        .insert(pid("buyer"), PlayerInventory::default());
    for item_def_id in ["wooden_shield", "dagger"] {
        game_state
            .offer_deal(
                &pid("npc_rica"),
                &pid("buyer"),
                item_def_id,
                DealKind::Buy,
                -10,
                "deal",
            )
            .await;
        game_state.clear_deal_cooldowns_for_test().await;
    }

    // 100 copper buys neither line.
    game_state
        .buy_items(
            &pid("buyer"),
            &pid("npc_rica"),
            vec![
                TradeLineItem {
                    item_def_id: "wooden_shield".to_string(),
                    qty: 1,
                },
                TradeLineItem {
                    item_def_id: "dagger".to_string(),
                    qty: 1,
                },
            ],
        )
        .await;

    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 100);
    assert!(game_state.inventories.read().await[&pid("buyer")]
        .bag
        .is_empty());
    assert_eq!(
        game_state
            .active_deals_for(&pid("buyer"), "Rica")
            .await
            .len(),
        2,
        "both deals must survive the abort"
    );
}

/// Within one batch a deal is redeemed by exactly one unit, and only the first
/// line touching that def.
#[tokio::test]
async fn buy_items_batch_redeems_each_deal_once() {
    let game_state = make_test_game_state("batch_buy_deal_once");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 30_000).await;
    game_state
        .inventories
        .write()
        .await
        .insert(pid("buyer"), PlayerInventory::default());
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

    // 3 shields at 2500: one at -10% (2250) plus two at full price.
    game_state
        .buy_items(
            &pid("buyer"),
            &pid("npc_rica"),
            vec![
                TradeLineItem {
                    item_def_id: "wooden_shield".to_string(),
                    qty: 2,
                },
                TradeLineItem {
                    item_def_id: "wooden_shield".to_string(),
                    qty: 1,
                },
            ],
        )
        .await;

    assert_eq!(
        game_state.get_player_gold(&pid("buyer")).await,
        30_000 - (2_250 + 2_500 + 2_500)
    );
    assert!(game_state
        .active_deals_for(&pid("buyer"), "Rica")
        .await
        .is_empty());
}

/// Recording a whole batch at once must trim to `BUYBACK_CAP` exactly as the
/// per-unit path did: oldest dropped first, order preserved.
#[tokio::test]
async fn sell_items_batch_keeps_only_the_newest_buyback_entries() {
    let game_state = make_test_game_state("batch_buyback_cap");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    game_state.inventories.write().await.insert(
        pid("buyer"),
        PlayerInventory {
            bag: vec![bag_item(7, "iron_sword", 8), bag_item(8, "dagger", 6)],
            ..Default::default()
        },
    );

    // 14 units in one call, cap 10: the 4 oldest (swords) fall off.
    game_state
        .sell_items(
            &pid("buyer"),
            &pid("npc_rica"),
            vec![
                BagLineItem {
                    instance_id: 7,
                    qty: 8,
                },
                BagLineItem {
                    instance_id: 8,
                    qty: 6,
                },
            ],
        )
        .await;

    let buybacks = game_state.buybacks.read().await;
    let list = &buybacks[&(1, "Rica".to_string())];
    assert_eq!(list.len(), 10);
    assert_eq!(
        list.iter()
            .filter(|s| s.entry.item_def_id == "iron_sword")
            .count(),
        4,
        "the oldest four sword entries are the ones dropped"
    );
    assert_eq!(
        list.iter()
            .filter(|s| s.entry.item_def_id == "dagger")
            .count(),
        6
    );
    let ids: Vec<u64> = list.iter().map(|s| s.entry.entry_id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "kept in sale order");
}

#[tokio::test]
async fn buy_items_batch_notifies_the_merchant_once_per_unit() {
    let game_state = make_test_game_state("batch_buy_notices");
    game_state
        .add_player(make_npc("npc_rica", "Rica", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("buyer", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("buyer"), 1, 0, attrs_with_cha(10), 10_000, None)
        .await;
    game_state
        .inventories
        .write()
        .await
        .insert(pid("buyer"), PlayerInventory::default());
    let mut npc_rx = game_state.register_direct_channel(&pid("npc_rica")).await;

    game_state
        .buy_items(
            &pid("buyer"),
            &pid("npc_rica"),
            vec![TradeLineItem {
                item_def_id: "healing_potion".to_string(),
                qty: 3,
            }],
        )
        .await;

    for _ in 0..3 {
        match npc_rx.try_recv() {
            Ok(ServerMessage::TradeNotice {
                item_def_id,
                kind: DealKind::Buy,
                price,
                ..
            }) => {
                assert_eq!(item_def_id, "healing_potion");
                assert_eq!(price, 600);
            }
            other => panic!("Expected per-unit TradeNotice, got {other:?}"),
        }
    }
    assert!(npc_rx.try_recv().is_err());
}

#[tokio::test]
async fn drop_items_batch_drops_partial_quantity_and_spawns_one_ground_item_per_unit() {
    let game_state = make_test_game_state("batch_drop_partial");
    game_state.add_player(make_player("owner", 1.0, 0.0)).await;
    game_state.inventories.write().await.insert(
        pid("owner"),
        PlayerInventory {
            bag: vec![bag_item(20, "healing_potion", 5), bag_item(21, "torch", 1)],
            ..Default::default()
        },
    );

    game_state
        .drop_items(
            &pid("owner"),
            vec![
                BagLineItem {
                    instance_id: 20,
                    qty: 3,
                },
                BagLineItem {
                    instance_id: 21,
                    qty: 1,
                },
            ],
        )
        .await;

    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&pid("owner")].bag;
    assert_eq!(bag.len(), 1, "the torch stack is fully consumed");
    assert_eq!(bag[0].item_def_id, "healing_potion");
    assert_eq!(bag[0].quantity, 2);
    drop(inventories);

    let ground_items = game_state.ground_items.read().await;
    assert_eq!(
        ground_items.len(),
        4,
        "3 potions + 1 torch land as 4 separate ground items"
    );
    let potions = ground_items
        .values()
        .filter(|gi| gi.item.item_def_id == "healing_potion")
        .count();
    assert_eq!(potions, 3);
}

#[tokio::test]
async fn drop_items_batch_is_all_or_nothing_when_quantity_exceeds_the_stack() {
    let game_state = make_test_game_state("batch_drop_atomic");
    game_state.add_player(make_player("owner", 1.0, 0.0)).await;
    game_state.inventories.write().await.insert(
        pid("owner"),
        PlayerInventory {
            bag: vec![bag_item(30, "healing_potion", 2)],
            ..Default::default()
        },
    );
    let mut owner_rx = game_state.register_direct_channel(&pid("owner")).await;

    game_state
        .drop_items(
            &pid("owner"),
            vec![BagLineItem {
                instance_id: 30,
                qty: 5,
            }],
        )
        .await;

    let inventories = game_state.inventories.read().await;
    assert_eq!(
        inventories[&pid("owner")].bag[0].quantity,
        2,
        "nothing dropped"
    );
    drop(inventories);
    assert!(game_state.ground_items.read().await.is_empty());
    match owner_rx.try_recv() {
        Ok(ServerMessage::SystemMessage { .. }) => {}
        other => panic!("Expected SystemMessage, got {:?}", other),
    }
}

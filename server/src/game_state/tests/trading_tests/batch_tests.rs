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
    game_state
        .add_player(make_npc("npc_karl", "Karl", 0.0, 0.0))
        .await;
    game_state.add_player(make_player("seller", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("seller"), 1, 0, attrs_with_cha(10), 0, None)
        .await;
    game_state
        .register_player_character(&pid("npc_karl"), 2, 0, attrs_with_cha(10), 100, None)
        .await;
    // Torches aren't stackable, so two purchased separately stay as two
    // instances; Karl's wishlist pays 60 each (base 50 @ 120%) — 120 total,
    // more than his 100-copper wallet.
    game_state.inventories.write().await.insert(
        pid("seller"),
        PlayerInventory {
            bag: vec![bag_item(1, "torch", 1), bag_item(2, "torch", 1)],
            ..Default::default()
        },
    );
    game_state
        .inventories
        .write()
        .await
        .insert(pid("npc_karl"), PlayerInventory::default());

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

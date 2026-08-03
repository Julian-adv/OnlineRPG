use super::*;

#[tokio::test]
async fn sell_to_merchant_records_buyback_and_restores_item() {
    let game_state = make_test_game_state("buyback_roundtrip");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(ItemInstance {
            instance_id: 7,
            item_def_id: "iron_sword".to_string(),
            quantity: 1,
            enchant: 2,
            durability: None,
        });
        inventories.insert(pid("buyer"), inv);
    }

    // Sell rate 40%: 10000 → 4000 payout, recorded for buyback.
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 4_000);
    let entry = {
        let buybacks = game_state.buybacks.read().await;
        let list = &buybacks[&(1, "Rica".to_string())];
        assert_eq!(list.len(), 1);
        list[0].entry.clone()
    };
    assert_eq!(entry.price, 4_000);
    assert_eq!(entry.enchant, 2);

    // Buying back costs the exact payout and restores the enchanted unit.
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry.entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 0);
    {
        let inventories = game_state.inventories.read().await;
        let bag = &inventories[&pid("buyer")].bag;
        assert_eq!(bag.len(), 1);
        assert_eq!(bag[0].item_def_id, "iron_sword");
        assert_eq!(bag[0].enchant, 2);
    }
    let buybacks = game_state.buybacks.read().await;
    assert!(buybacks
        .get(&(1, "Rica".to_string()))
        .is_none_or(|list| list.is_empty()));
}

#[tokio::test]
async fn merchant_buyback_preserves_damaged_armor_condition() {
    let game_state = make_test_game_state("buyback_durability_roundtrip");
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
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry = game_state.buybacks.read().await[&(1, "Rica".to_string())][0]
        .entry
        .clone();
    assert_eq!(entry.durability, Some(17));

    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry.entry_id)
        .await;
    assert_eq!(
        game_state.inventories.read().await[&pid("buyer")].bag[0].durability,
        Some(17)
    );
}

#[tokio::test]
async fn buyback_rejects_without_enough_gold() {
    let game_state = make_test_game_state("buyback_no_gold");
    let (mut buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }

    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };
    {
        let mut gold = game_state.player_gold.write().await;
        *gold.get_mut(&pid("buyer")).unwrap() = 3_999;
    }

    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 3_999);
    {
        let inventories = game_state.inventories.read().await;
        assert!(inventories[&pid("buyer")].bag.is_empty());
    }
    // The entry survives a failed buyback.
    let buybacks = game_state.buybacks.read().await;
    assert_eq!(buybacks[&(1, "Rica".to_string())].len(), 1);
    drop(buybacks);
    for msg in drain(&mut buyer_rx) {
        if let ServerMessage::TradeError { message } = msg {
            assert_eq!(message, "Not enough gold");
            return;
        }
    }
    panic!("expected a TradeError");
}

#[tokio::test]
async fn buyback_list_keeps_only_the_newest_entries() {
    let game_state = make_test_game_state("buyback_cap");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 12));
        inventories.insert(pid("buyer"), inv);
    }

    for _ in 0..12 {
        game_state
            .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
            .await;
    }
    let buybacks = game_state.buybacks.read().await;
    assert_eq!(buybacks[&(1, "Rica".to_string())].len(), 10);
}

/// Backdate every stored entry so it has already expired.
async fn expire_all_buybacks(game_state: &GameState) {
    let mut buybacks = game_state.buybacks.write().await;
    for list in buybacks.values_mut() {
        for stored in list.iter_mut() {
            stored.expires_at_ms = 0;
        }
    }
}

#[tokio::test]
async fn expired_buyback_entries_are_swept_with_their_pair() {
    let game_state = make_test_game_state("buyback_expiry_sweep");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    assert_eq!(game_state.buybacks.read().await.len(), 1);

    expire_all_buybacks(&game_state).await;
    // Opening the shop sweeps globally, so the whole pair goes — otherwise an
    // offline character's entries would never be reached again.
    game_state
        .open_shop(&pid("buyer"), &pid("npc_rica"), true)
        .await;
    assert!(game_state.buybacks.read().await.is_empty());
}

#[tokio::test]
async fn expired_buyback_cannot_be_repurchased() {
    let game_state = make_test_game_state("buyback_expiry_reject");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };
    let gold_after_sell = game_state.get_player_gold(&pid("buyer")).await;

    expire_all_buybacks(&game_state).await;
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;

    // Rejected: no item restored and no gold taken.
    assert_eq!(
        game_state.get_player_gold(&pid("buyer")).await,
        gold_after_sell
    );
    assert!(game_state.inventories.read().await[&pid("buyer")]
        .bag
        .is_empty());
}

#[tokio::test]
async fn buyback_survives_a_reconnect() {
    let game_state = make_test_game_state("buyback_reconnect");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };

    // Disconnect: the session's player id and registration go away.
    game_state.remove_player(&pid("buyer")).await;
    game_state.unregister_player_character(&pid("buyer")).await;

    // Reconnect under a fresh player id but the same character (id 1).
    game_state.add_player(make_player("buyer2", 1.0, 0.0)).await;
    game_state
        .register_player_character(&pid("buyer2"), 1, 0, attrs_with_cha(10), 4_000, None)
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("buyer2"), Default::default());
    }

    game_state
        .buyback_item(&pid("buyer2"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer2")).await, 0);
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("buyer2")].bag[0].item_def_id, "iron_sword");
}

/// The entry is consumed inside the gold/inventory critical section, so two
/// requests racing for one entry_id must restore one unit, not duplicate it.
#[tokio::test]
async fn concurrent_buybacks_of_one_entry_restore_a_single_unit() {
    let game_state = make_test_game_state("buyback_race");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };
    // Fund a second purchase, so only the entry consumption — not the gold
    // check — can stop a duplicate.
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 4_000);
    game_state
        .player_gold
        .write()
        .await
        .insert(pid("buyer"), 8_000);

    let (buyer, npc) = (pid("buyer"), pid("npc_rica"));
    tokio::join!(
        game_state.buyback_item(&buyer, &npc, entry_id),
        game_state.buyback_item(&buyer, &npc, entry_id),
    );

    // One unit back, paid for once — the loser is rejected, not served.
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("buyer")].bag.len(), 1);
    drop(inventories);
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 4_000);
    // The loser's sweep drops the pair once its last entry is consumed.
    assert!(game_state
        .buybacks
        .read()
        .await
        .get(&(1, "Rica".to_string()))
        .is_none_or(|list| list.is_empty()));
}

#[tokio::test]
async fn buyback_after_haggled_sell_is_gold_neutral() {
    let game_state = make_test_game_state("buyback_deal_neutral");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 18, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
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

    // Boosted payout: 10000 * 0.4 * 1.25 = 5000. Buying back costs the
    // same 5000, so the deal cannot be turned into a money pump.
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 5_000);
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        let entry = &buybacks[&(1, "Rica".to_string())][0].entry;
        assert_eq!(entry.price, 5_000);
        entry.entry_id
    };
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, 0);
    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("buyer")].bag.len(), 1);
}

#[tokio::test]
async fn buyback_rejects_when_too_heavy_and_keeps_the_entry() {
    let game_state = make_test_game_state("buyback_overweight");
    let (mut buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 100_000).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };

    // Fill the bag right up to the carry limit so the returning sword
    // cannot fit.
    {
        let max_weight = game_state.max_carry_weight(&pid("buyer")).await;
        let sword_weight = game_state.item_defs.weight("iron_sword");
        let fill = (max_weight / sword_weight) as u32;
        let mut inventories = game_state.inventories.write().await;
        inventories.get_mut(&pid("buyer")).unwrap().bag = vec![bag_item(8, "iron_sword", fill)];
    }

    let gold_before = game_state.get_player_gold(&pid("buyer")).await;
    game_state
        .buyback_item(&pid("buyer"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("buyer")).await, gold_before);
    let buybacks = game_state.buybacks.read().await;
    assert_eq!(buybacks[&(1, "Rica".to_string())].len(), 1);
    drop(buybacks);
    for msg in drain(&mut buyer_rx) {
        if let ServerMessage::TradeError { message } = msg {
            assert_eq!(message, "Too heavy to carry");
            return;
        }
    }
    panic!("expected a TradeError");
}

#[tokio::test]
async fn buyback_is_scoped_to_the_selling_character() {
    let game_state = make_test_game_state("buyback_scoped");
    let (_buyer_rx, _npc_rx) = setup_haggle(&game_state, 10, 0).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(7, "iron_sword", 1));
        inventories.insert(pid("buyer"), inv);
    }
    game_state
        .sell_item(&pid("buyer"), &pid("npc_rica"), 7)
        .await;
    let entry_id = {
        let buybacks = game_state.buybacks.read().await;
        buybacks[&(1, "Rica".to_string())][0].entry.entry_id
    };

    // A different character (id 2) cannot take the seller's entry, even
    // with the exact entry id and enough gold.
    game_state.add_player(make_player("other", 1.0, 0.5)).await;
    game_state
        .register_player_character(&pid("other"), 2, 0, attrs_with_cha(10), 100_000, None)
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("other"), Default::default());
    }
    let mut other_rx = game_state.register_direct_channel(&pid("other")).await;
    game_state
        .buyback_item(&pid("other"), &pid("npc_rica"), entry_id)
        .await;
    assert_eq!(game_state.get_player_gold(&pid("other")).await, 100_000);
    let buybacks = game_state.buybacks.read().await;
    assert_eq!(buybacks[&(1, "Rica".to_string())].len(), 1);
    drop(buybacks);
    for msg in drain(&mut other_rx) {
        if let ServerMessage::TradeError { message } = msg {
            assert_eq!(message, "That item is no longer available");
            return;
        }
    }
    panic!("expected a TradeError");
}

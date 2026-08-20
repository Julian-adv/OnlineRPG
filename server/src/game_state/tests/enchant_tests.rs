use super::*;

// --- Enchant weapon scrolls ---

/// Instance id the scroll stack takes in every setup below; the equipped
/// pieces number up from 1.
const SCROLL_ID: u64 = 9;

/// Instance id of the whetstone oil every reading burns alongside the scroll.
const OIL_ID: u64 = 10;

/// Spawn a live player wearing `equipped` (slot, def id, enchant) with a stack
/// of `scroll_def_id` and `oil` flasks of whetstone oil in the bag, and return
/// their direct channel.
async fn setup_enchant_reader(
    game_state: &GameState,
    equipped: &[(EquipSlot, &str, i32)],
    scroll_def_id: &str,
    scrolls: u32,
    oil: u32,
) -> DirectRx {
    game_state.add_player(make_player("reader", 0.0, 0.0)).await;
    let rx = game_state.register_direct_channel(&pid("reader")).await;

    let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
    for (index, (slot, item_def_id, enchant)) in equipped.iter().enumerate() {
        inv.equipped.insert(
            *slot,
            ItemInstance {
                instance_id: index as u64 + 1,
                item_def_id: item_def_id.to_string(),
                quantity: 1,
                enchant: *enchant,
                cape_color: None,
                cape_texture: None,
            },
        );
    }
    inv.bag.push(bag_item(SCROLL_ID, scroll_def_id, scrolls));
    if oil > 0 {
        inv.bag.push(bag_item(OIL_ID, "whetstone_oil", oil));
    }
    game_state
        .inventories
        .write()
        .await
        .insert(pid("reader"), inv);
    rx
}

/// The weapon-scroll setup: a wielded `weapon` at an enchant level, or nothing
/// in hand.
async fn setup_weapon_enchant_reader(
    game_state: &GameState,
    weapon: Option<(&str, i32)>,
    scrolls: u32,
) -> DirectRx {
    let equipped: Vec<(EquipSlot, &str, i32)> = weapon
        .map(|(def_id, enchant)| (EquipSlot::MainHand, def_id, enchant))
        .into_iter()
        .collect();
    setup_enchant_reader(
        game_state,
        &equipped,
        "scroll_of_enchant_weapon",
        scrolls,
        scrolls,
    )
    .await
}

#[tokio::test]
async fn enchant_scroll_enchants_wielded_weapon() {
    let game_state = make_test_game_state("enchant_ok");
    let _rx = setup_weapon_enchant_reader(&game_state, Some(("iron_sword", 0)), 1).await;

    game_state.use_item(&pid("reader"), SCROLL_ID).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    let weapon = inv.equipped.get(&EquipSlot::MainHand).unwrap();
    assert_eq!(weapon.enchant, 1);
    assert!(inv.bag.is_empty(), "the scroll and the oil should be spent");
}

#[tokio::test]
async fn enchant_scroll_requires_wielded_weapon() {
    let game_state = make_test_game_state("enchant_no_weapon");
    let mut rx = setup_weapon_enchant_reader(&game_state, None, 1).await;

    game_state.use_item(&pid("reader"), SCROLL_ID).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    assert_eq!(inv.bag.len(), 2, "the scroll and the oil should be kept");
    match rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(
                message.contains("no weapon"),
                "unexpected message: {message}"
            );
        }
        other => panic!("Expected a system reply, got {:?}", other),
    }
}

// --- Enchant armor scrolls ---

/// The armor-scroll setup: whatever gear the case needs worn.
async fn setup_armor_enchant_reader(
    game_state: &GameState,
    armor: &[(EquipSlot, &str, i32)],
    scrolls: u32,
) -> DirectRx {
    setup_enchant_reader(
        game_state,
        armor,
        "scroll_of_enchant_armor",
        scrolls,
        scrolls,
    )
    .await
}

#[tokio::test]
async fn enchant_armor_scroll_enchants_worn_armor() {
    let game_state = make_test_game_state("enchant_armor_ok");
    let _rx =
        setup_armor_enchant_reader(&game_state, &[(EquipSlot::Chest, "leather_armor", 0)], 1).await;

    let base_guard = game_state.effective_guard(&pid("reader")).await;
    game_state.use_item(&pid("reader"), SCROLL_ID).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    assert_eq!(inv.equipped.get(&EquipSlot::Chest).unwrap().enchant, 1);
    assert!(inv.bag.is_empty(), "the scroll and the oil should be spent");
    assert_eq!(
        game_state.effective_guard(&pid("reader")).await,
        base_guard + 1,
        "the enchant should raise guard"
    );
}

#[tokio::test]
async fn enchant_armor_scroll_ignores_weapons_and_accessories() {
    let game_state = make_test_game_state("enchant_armor_targets");
    let _rx = setup_armor_enchant_reader(
        &game_state,
        &[
            (EquipSlot::MainHand, "iron_sword", 0),
            (EquipSlot::Ring, "ring_of_protection", 0),
            (EquipSlot::Chest, "leather_armor", 0),
        ],
        1,
    )
    .await;

    game_state.use_item(&pid("reader"), SCROLL_ID).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    assert_eq!(inv.equipped.get(&EquipSlot::Chest).unwrap().enchant, 1);
    assert_eq!(inv.equipped.get(&EquipSlot::MainHand).unwrap().enchant, 0);
    assert_eq!(inv.equipped.get(&EquipSlot::Ring).unwrap().enchant, 0);
}

#[tokio::test]
async fn enchant_armor_scroll_requires_worn_armor() {
    let game_state = make_test_game_state("enchant_armor_none");
    let mut rx =
        setup_armor_enchant_reader(&game_state, &[(EquipSlot::MainHand, "iron_sword", 0)], 1).await;

    game_state.use_item(&pid("reader"), SCROLL_ID).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    assert_eq!(inv.bag.len(), 2, "the scroll and the oil should be kept");
    match rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(
                message.contains("no armor"),
                "unexpected message: {message}"
            );
        }
        other => panic!("Expected a system reply, got {:?}", other),
    }
}

#[tokio::test]
async fn enchant_armor_scroll_destroys_over_enchanted_armor() {
    let game_state = make_test_game_state("enchant_armor_boom");
    let _rx =
        setup_armor_enchant_reader(&game_state, &[(EquipSlot::Chest, "leather_armor", 12)], 100)
            .await;

    let reader = pid("reader");
    for _ in 0..100 {
        game_state.use_item(&reader, SCROLL_ID).await;
        let inv = game_state.get_player_inventory(&reader).await.unwrap();
        if !inv.equipped.contains_key(&EquipSlot::Chest) {
            return; // evaporated, as expected
        }
    }
    panic!("the armor should have evaporated within 100 reads at 99% odds");
}

#[tokio::test]
async fn enchant_scroll_destroys_over_enchanted_weapon() {
    let game_state = make_test_game_state("enchant_boom");
    // At +12 the success floor is 1%, so each read is a 99% destruction
    // roll. 100 scrolls make survival odds ~1e-200: the loop below is
    // deterministic for all practical purposes.
    let _rx = setup_weapon_enchant_reader(&game_state, Some(("iron_sword", 12)), 100).await;

    let reader = pid("reader");
    for _ in 0..100 {
        game_state.use_item(&reader, SCROLL_ID).await;
        let inv = game_state.get_player_inventory(&reader).await.unwrap();
        if !inv.equipped.contains_key(&EquipSlot::MainHand) {
            return; // evaporated, as expected
        }
    }
    panic!("the weapon should have evaporated within 100 reads at 99% odds");
}

#[tokio::test]
async fn enchanting_needs_whetstone_oil() {
    let game_state = make_test_game_state("enchant_no_oil");
    let mut rx = setup_enchant_reader(
        &game_state,
        &[(EquipSlot::MainHand, "iron_sword", 0)],
        "scroll_of_enchant_weapon",
        1,
        0,
    )
    .await;

    game_state.use_item(&pid("reader"), SCROLL_ID).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    assert_eq!(inv.equipped.get(&EquipSlot::MainHand).unwrap().enchant, 0);
    assert_eq!(inv.bag.len(), 1, "the scroll should be kept");
    match rx.try_recv() {
        Ok(ServerMessage::SystemMessage { message }) => {
            assert!(message.contains("Oil"), "unexpected message: {message}");
        }
        other => panic!("Expected a system reply, got {:?}", other),
    }
}

#[tokio::test]
async fn each_reading_burns_one_flask_of_oil() {
    let game_state = make_test_game_state("enchant_oil_spend");
    let _rx = setup_weapon_enchant_reader(&game_state, Some(("iron_sword", 0)), 3).await;

    game_state.use_item(&pid("reader"), SCROLL_ID).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    let oil = inv
        .bag
        .iter()
        .find(|item| item.item_def_id == "whetstone_oil")
        .expect("the oil stack should survive a single reading");
    assert_eq!(oil.quantity, 2, "one flask per reading");
}

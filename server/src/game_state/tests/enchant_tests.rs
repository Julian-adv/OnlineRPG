use super::*;

// --- Enchant weapon scrolls ---

/// Spawn a live player wielding `weapon` at the given enchant level with one
/// enchant scroll (instance 2) in the bag, and return their direct channel.
async fn setup_enchant_reader(
    game_state: &GameState,
    weapon: Option<(&str, i32)>,
    scrolls: u32,
) -> DirectRx {
    game_state.add_player(make_player("reader", 0.0, 0.0)).await;
    let rx = game_state.register_direct_channel(&pid("reader")).await;

    let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
    if let Some((weapon_def_id, enchant)) = weapon {
        inv.equipped.insert(
            EquipSlot::MainHand,
            ItemInstance {
                instance_id: 1,
                item_def_id: weapon_def_id.to_string(),
                quantity: 1,
                enchant,
                durability: None,
            },
        );
    }
    inv.bag
        .push(bag_item(2, "scroll_of_enchant_weapon", scrolls));
    game_state
        .inventories
        .write()
        .await
        .insert(pid("reader"), inv);
    rx
}

#[tokio::test]
async fn enchant_scroll_enchants_wielded_weapon() {
    let game_state = make_test_game_state("enchant_ok");
    let _rx = setup_enchant_reader(&game_state, Some(("iron_sword", 0)), 1).await;

    game_state.use_item(&pid("reader"), 2).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    let weapon = inv.equipped.get(&EquipSlot::MainHand).unwrap();
    assert_eq!(weapon.enchant, 1);
    assert!(inv.bag.is_empty(), "the scroll should be consumed");
}

#[tokio::test]
async fn enchant_scroll_requires_wielded_weapon() {
    let game_state = make_test_game_state("enchant_no_weapon");
    let mut rx = setup_enchant_reader(&game_state, None, 1).await;

    game_state.use_item(&pid("reader"), 2).await;

    let inv = game_state
        .get_player_inventory(&pid("reader"))
        .await
        .unwrap();
    assert_eq!(inv.bag.len(), 1, "the scroll should be kept");
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

#[tokio::test]
async fn enchant_scroll_destroys_over_enchanted_weapon() {
    let game_state = make_test_game_state("enchant_boom");
    // At +12 the success floor is 1%, so each read is a 99% destruction
    // roll. 100 scrolls make survival odds ~1e-200: the loop below is
    // deterministic for all practical purposes.
    let _rx = setup_enchant_reader(&game_state, Some(("iron_sword", 12)), 100).await;

    let reader = pid("reader");
    for _ in 0..100 {
        game_state.use_item(&reader, 2).await;
        let inv = game_state.get_player_inventory(&reader).await.unwrap();
        if !inv.equipped.contains_key(&EquipSlot::MainHand) {
            return; // evaporated, as expected
        }
    }
    panic!("the weapon should have evaporated within 100 reads at 99% odds");
}

use super::*;

/// Bag instance the transfer kit takes in every setup below.
const KIT_ID: u64 = 8;
const CAPE_ID: u64 = 1;
/// A well-formed content hash. The store only ever answers with hashes of
/// this shape, and only a file that exists under one is wearable.
const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A live player with `kits` transfer kits in the bag, wearing a cape if
/// `caped`, and one texture already sitting in the store.
async fn setup_printer(game_state: &GameState, caped: bool, kits: u32) -> DirectRx {
    game_state
        .add_player(make_player("printer", 0.0, 0.0))
        .await;
    let rx = game_state.register_direct_channel(&pid("printer")).await;

    let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
    if caped {
        inv.equipped
            .insert(EquipSlot::Back, bag_item(CAPE_ID, "wool_cape", 1));
    }
    if kits > 0 {
        inv.bag.push(bag_item(KIT_ID, "cape_transfer_kit", kits));
    }
    game_state
        .inventories
        .write()
        .await
        .insert(pid("printer"), inv);
    std::fs::write(game_state.cape_textures().path(HASH), b"png").expect("stored texture");
    rx
}

async fn worn_cape(game_state: &GameState) -> ItemInstance {
    game_state
        .get_player_inventory(&pid("printer"))
        .await
        .unwrap()
        .equipped
        .remove(&EquipSlot::Back)
        .unwrap()
}

#[tokio::test]
async fn using_a_kit_prompts_for_a_picture_and_spends_nothing() {
    let game_state = make_test_game_state("print_prompt");
    let mut rx = setup_printer(&game_state, true, 1).await;

    game_state.use_item(&pid("printer"), KIT_ID).await;

    let prompted = drain(&mut rx).into_iter().any(|msg| {
        matches!(msg, ServerMessage::CapeTexturePrompt { instance_id } if instance_id == KIT_ID)
    });
    assert!(prompted, "the client should be asked for a picture");
    let inv = game_state
        .get_player_inventory(&pid("printer"))
        .await
        .unwrap();
    assert_eq!(inv.bag[0].quantity, 1, "the prompt spends nothing");
}

#[tokio::test]
async fn applying_a_stored_texture_prints_the_cape_and_spends_the_kit() {
    let game_state = make_test_game_state("print_ok");
    let _rx = setup_printer(&game_state, true, 2).await;

    game_state
        .apply_cape_texture(&pid("printer"), KIT_ID, HASH)
        .await;

    assert_eq!(
        worn_cape(&game_state).await.cape_texture.as_deref(),
        Some(HASH)
    );
    let inv = game_state
        .get_player_inventory(&pid("printer"))
        .await
        .unwrap();
    assert_eq!(inv.bag[0].quantity, 1, "one kit should be spent");
}

/// The hash is a client-supplied string; anything the store cannot vouch for
/// must not reach other players' clients as a URL.
#[tokio::test]
async fn a_texture_the_store_never_saw_is_refused() {
    let game_state = make_test_game_state("print_unknown_hash");
    let _rx = setup_printer(&game_state, true, 1).await;

    for bad in [
        "../../etc/passwd",
        "not-a-hash",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    ] {
        game_state
            .apply_cape_texture(&pid("printer"), KIT_ID, bad)
            .await;
    }

    assert_eq!(worn_cape(&game_state).await.cape_texture, None);
    let inv = game_state
        .get_player_inventory(&pid("printer"))
        .await
        .unwrap();
    assert_eq!(inv.bag[0].quantity, 1, "the kit should be kept");
}

#[tokio::test]
async fn printing_without_a_cape_keeps_the_kit() {
    let game_state = make_test_game_state("print_no_cape");
    let mut rx = setup_printer(&game_state, false, 1).await;

    game_state.use_item(&pid("printer"), KIT_ID).await;
    game_state
        .apply_cape_texture(&pid("printer"), KIT_ID, HASH)
        .await;

    let inv = game_state
        .get_player_inventory(&pid("printer"))
        .await
        .unwrap();
    assert_eq!(inv.bag[0].quantity, 1, "the kit should be kept");
    let refusals = drain(&mut rx)
        .into_iter()
        .filter(|msg| {
            matches!(msg, ServerMessage::SystemMessage { message } if message.contains("not wearing a cape"))
        })
        .count();
    assert_eq!(refusals, 2, "both the prompt and the print should refuse");
}

/// Printing leaves the def id and the dye alone, so a comparison that only
/// watches those would drop this.
#[tokio::test]
async fn a_print_reaches_nearby_players() {
    let game_state = make_test_game_state("print_broadcast");
    let _rx = setup_printer(&game_state, true, 1).await;
    game_state
        .add_player(make_player("watcher", 2.0, 0.0))
        .await;
    let worn = game_state
        .get_player_inventory(&pid("printer"))
        .await
        .expect("the printer has an inventory");
    game_state.set_player_gear(&pid("printer"), &worn).await;
    let mut watcher_rx = game_state.register_direct_channel(&pid("watcher")).await;

    game_state
        .apply_cape_texture(&pid("printer"), KIT_ID, HASH)
        .await;

    let printed = drain(&mut watcher_rx).into_iter().any(|msg| {
        matches!(msg, ServerMessage::PlayerBackChanged { player_id, cape_texture, .. }
            if player_id == pid("printer") && cape_texture.as_deref() == Some(HASH))
    });
    assert!(printed, "nearby players must see the new print");
}

#[tokio::test]
async fn a_dropped_cape_keeps_its_print() {
    let game_state = make_test_game_state("print_survives_drop");
    let _rx = setup_printer(&game_state, true, 1).await;
    game_state
        .apply_cape_texture(&pid("printer"), KIT_ID, HASH)
        .await;

    game_state.drop_item(&pid("printer"), CAPE_ID).await;
    let dropped = game_state
        .ground_items
        .read()
        .await
        .values()
        .map(|entry| entry.item.clone())
        .find(|item| item.item_def_id == "wool_cape")
        .expect("the cape should be on the ground");
    assert_eq!(dropped.cape_texture.as_deref(), Some(HASH));

    game_state
        .pickup_item(&pid("printer"), dropped.instance_id)
        .await;
    let inv = game_state
        .get_player_inventory(&pid("printer"))
        .await
        .unwrap();
    let cape = inv
        .bag
        .iter()
        .find(|item| item.item_def_id == "wool_cape")
        .expect("the cape should be back in the bag");
    assert_eq!(cape.cape_texture.as_deref(), Some(HASH));
}

/// Two capes that differ only in their print are different objects; merging
/// them would hand one player's picture to another.
#[tokio::test]
async fn prints_keep_otherwise_identical_capes_apart() {
    let game_state = make_test_game_state("print_no_merge");
    let mut bag = Vec::new();
    for (id, texture) in [(1u64, Some(HASH.to_string())), (2, None)] {
        super::super::inventory::stack_into_bag(
            &mut bag,
            super::super::inventory::BagInsert {
                stackable: true,
                item_def_id: "wool_cape",
                enchant: 0,
                cape_color: None,
                cape_texture: texture,
                first_instance_id: id,
                quantity: 1,
            },
        );
    }
    drop(game_state);
    assert_eq!(
        bag.len(),
        2,
        "a printed cape must not merge with a plain one"
    );
}

/// Blocking is by hash: the file goes, the fetch 404s, and the same picture
/// cannot be uploaded back in.
#[tokio::test]
async fn blocking_a_hash_unwears_it_everywhere() {
    let game_state = make_test_game_state("print_block");
    let _rx = setup_printer(&game_state, true, 1).await;
    let store = game_state.cape_textures();
    assert!(store.is_wearable(HASH).await);

    store.block(HASH).await.expect("blocking works");

    assert!(!store.is_wearable(HASH).await);
    assert!(!store.path(HASH).exists(), "the file should be gone");
    assert!(store.is_blocked(HASH).await);
}

#[tokio::test]
async fn reporting_a_bare_cape_says_so() {
    let game_state = make_test_game_state("print_report");
    let mut rx = setup_printer(&game_state, true, 1).await;
    game_state
        .add_player(make_player("watcher", 2.0, 0.0))
        .await;

    game_state
        .report_cape_texture(&pid("printer"), &pid("watcher"))
        .await;

    let told = drain(&mut rx).into_iter().any(|msg| {
        matches!(msg, ServerMessage::SystemMessage { message } if message.contains("not wearing a printed cape"))
    });
    assert!(told, "there is nothing to report on a bare cape");
}

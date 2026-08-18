use super::*;

/// Bag instance the dye bottle takes in every setup below.
const DYE_ID: u64 = 7;
const CAPE_ID: u64 = 1;
const RED: &str = "#3355ff";

/// A live player with `dyes` bottles in the bag, wearing a cape if `caped`.
async fn setup_dyer(game_state: &GameState, caped: bool, dyes: u32) -> DirectRx {
    game_state.add_player(make_player("dyer", 0.0, 0.0)).await;
    let rx = game_state.register_direct_channel(&pid("dyer")).await;

    let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
    if caped {
        inv.equipped
            .insert(EquipSlot::Back, bag_item(CAPE_ID, "wool_cape", 1));
    }
    if dyes > 0 {
        inv.bag.push(bag_item(DYE_ID, "cape_dye", dyes));
    }
    game_state
        .inventories
        .write()
        .await
        .insert(pid("dyer"), inv);
    rx
}

async fn worn_cape(game_state: &GameState) -> ItemInstance {
    game_state
        .get_player_inventory(&pid("dyer"))
        .await
        .unwrap()
        .equipped
        .remove(&EquipSlot::Back)
        .unwrap()
}

#[tokio::test]
async fn using_a_dye_prompts_for_a_colour_and_spends_nothing() {
    let game_state = make_test_game_state("dye_prompt");
    let mut rx = setup_dyer(&game_state, true, 1).await;

    game_state.use_item(&pid("dyer"), DYE_ID).await;

    let prompted = drain(&mut rx).into_iter().any(
        |msg| matches!(msg, ServerMessage::CapeDyePrompt { instance_id } if instance_id == DYE_ID),
    );
    assert!(prompted, "the client should be asked for a colour");
    let inv = game_state.get_player_inventory(&pid("dyer")).await.unwrap();
    assert_eq!(inv.bag[0].quantity, 1, "the prompt spends nothing");
}

#[tokio::test]
async fn dye_recolours_the_worn_cape_and_is_spent() {
    let game_state = make_test_game_state("dye_ok");
    let _rx = setup_dyer(&game_state, true, 2).await;

    game_state.dye_cape(&pid("dyer"), DYE_ID, "#3355FF").await;

    assert_eq!(
        worn_cape(&game_state).await.cape_color.as_deref(),
        Some(RED)
    );
    let inv = game_state.get_player_inventory(&pid("dyer")).await.unwrap();
    assert_eq!(inv.bag[0].quantity, 1, "one bottle should be spent");
}

#[tokio::test]
async fn dyeing_without_a_cape_keeps_the_bottle() {
    let game_state = make_test_game_state("dye_no_cape");
    let mut rx = setup_dyer(&game_state, false, 1).await;

    game_state.use_item(&pid("dyer"), DYE_ID).await;
    game_state.dye_cape(&pid("dyer"), DYE_ID, RED).await;

    let inv = game_state.get_player_inventory(&pid("dyer")).await.unwrap();
    assert_eq!(inv.bag[0].quantity, 1, "the bottle should be kept");
    let refusals = drain(&mut rx)
        .into_iter()
        .filter(
            |msg| matches!(msg, ServerMessage::SystemMessage { message } if message.contains("not wearing a cape")),
        )
        .count();
    assert_eq!(refusals, 2, "both the prompt and the dye should refuse");
}

#[tokio::test]
async fn a_colour_that_is_not_a_colour_is_refused() {
    let game_state = make_test_game_state("dye_bad_colour");
    let _rx = setup_dyer(&game_state, true, 1).await;

    for bad in ["red", "#12345", "#12345g", "3355ff"] {
        game_state.dye_cape(&pid("dyer"), DYE_ID, bad).await;
    }

    assert_eq!(worn_cape(&game_state).await.cape_color, None);
    let inv = game_state.get_player_inventory(&pid("dyer")).await.unwrap();
    assert_eq!(inv.bag[0].quantity, 1, "the bottle should be kept");
}

#[tokio::test]
async fn a_recolour_reaches_nearby_players() {
    let game_state = make_test_game_state("dye_broadcast");
    let _rx = setup_dyer(&game_state, true, 1).await;
    game_state
        .add_player(make_player("watcher", 2.0, 0.0))
        .await;
    // The cape's def id does not change when it is dyed, so this is the case
    // a def-id-only change compare would silently drop.
    let worn = game_state
        .get_player_inventory(&pid("dyer"))
        .await
        .expect("the dyer has an inventory");
    game_state.set_player_gear(&pid("dyer"), &worn).await;
    let mut watcher_rx = game_state.register_direct_channel(&pid("watcher")).await;

    game_state.dye_cape(&pid("dyer"), DYE_ID, RED).await;

    let recoloured = drain(&mut watcher_rx).into_iter().any(|msg| {
        matches!(msg, ServerMessage::PlayerBackChanged { player_id, cape_color, .. }
            if player_id == pid("dyer") && cape_color.as_deref() == Some(RED))
    });
    assert!(recoloured, "nearby players must see the new colour");
}

#[tokio::test]
async fn taking_a_dyed_cape_off_and_on_keeps_the_dye() {
    let game_state = make_test_game_state("dye_survives_reequip");
    let _rx = setup_dyer(&game_state, true, 1).await;
    game_state.dye_cape(&pid("dyer"), DYE_ID, RED).await;
    game_state
        .add_player(make_player("watcher", 2.0, 0.0))
        .await;
    let mut watcher_rx = game_state.register_direct_channel(&pid("watcher")).await;

    game_state.unequip_item(&pid("dyer"), EquipSlot::Back).await;
    game_state.equip_item(&pid("dyer"), CAPE_ID).await;

    assert_eq!(
        worn_cape(&game_state).await.cape_color.as_deref(),
        Some(RED)
    );
    let redressed = drain(&mut watcher_rx).into_iter().any(|msg| {
        matches!(msg, ServerMessage::PlayerBackChanged { cape_color, item_def_id, .. }
            if item_def_id.is_some() && cape_color.as_deref() == Some(RED))
    });
    assert!(redressed, "putting it back on must broadcast the dye again");
}

#[tokio::test]
async fn a_dropped_cape_keeps_its_dye() {
    let game_state = make_test_game_state("dye_survives_drop");
    let _rx = setup_dyer(&game_state, true, 1).await;
    game_state.dye_cape(&pid("dyer"), DYE_ID, RED).await;

    game_state.drop_item(&pid("dyer"), CAPE_ID).await;
    let dropped = game_state
        .ground_items
        .read()
        .await
        .values()
        .map(|entry| entry.item.clone())
        .find(|item| item.item_def_id == "wool_cape")
        .expect("the cape should be on the ground");
    assert_eq!(dropped.cape_color.as_deref(), Some(RED));

    game_state
        .pickup_item(&pid("dyer"), dropped.instance_id)
        .await;
    let inv = game_state.get_player_inventory(&pid("dyer")).await.unwrap();
    let cape = inv
        .bag
        .iter()
        .find(|item| item.item_def_id == "wool_cape")
        .expect("the cape should be back in the bag");
    assert_eq!(cape.cape_color.as_deref(), Some(RED));
}

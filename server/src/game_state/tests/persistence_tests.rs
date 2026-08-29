use super::*;

#[tokio::test]
async fn dirty_save_is_retried_after_failure() {
    let (auth, db_path) = make_test_auth_with_path("dirty_save_retry");
    let account = auth.login_npc("npc_dirty_save_retry").unwrap();
    let attributes = attrs_with_cha(12);
    let record = create_test_character(&auth, &account, "Retryknight");
    let initial_inventory = auth.load_inventory(record.id).unwrap();

    let game_state = make_test_game_state("dirty_save_retry");
    let player_id = pid("dirty_save_retry");
    let mut player = make_player("dirty_save_retry", 42.0, 0.0);
    player.name = record.name.clone();
    game_state.add_player(player).await;
    game_state
        .register_player_character(
            &player_id,
            record.id,
            record.xp,
            attributes,
            record.gold,
            None,
        )
        .await;
    game_state.inventories.write().await.insert(
        player_id,
        PlayerInventory {
            bag: vec![bag_item(1, "torch", 1)],
            equipped: Default::default(),
        },
    );
    game_state.mark_dirty(&player_id).await;
    game_state.mark_inventory_dirty(&player_id).await;

    let failure_connection = rusqlite::Connection::open(&db_path).unwrap();
    failure_connection
        .execute_batch(
            "CREATE TRIGGER fail_character_update
             BEFORE UPDATE ON characters
             BEGIN
                 SELECT RAISE(FAIL, 'test save failure');
             END;",
        )
        .unwrap();

    game_state.flush_dirty_saves(&auth).await;

    assert!(game_state.dirty_players.read().await.contains(&player_id));
    assert!(game_state
        .dirty_inventories
        .read()
        .await
        .contains(&player_id));
    assert_ne!(
        auth.get_character_for_account(&account, record.id)
            .unwrap()
            .last_x,
        42.0
    );
    assert_eq!(auth.load_inventory(record.id).unwrap(), initial_inventory);

    failure_connection
        .execute("DROP TRIGGER fail_character_update", [])
        .unwrap();
    game_state.flush_dirty_saves(&auth).await;

    assert!(!game_state.dirty_players.read().await.contains(&player_id));
    assert!(!game_state
        .dirty_inventories
        .read()
        .await
        .contains(&player_id));
    assert_eq!(
        auth.get_character_for_account(&account, record.id)
            .unwrap()
            .last_x,
        42.0
    );
    assert_eq!(
        auth.load_inventory(record.id).unwrap(),
        vec![crate::auth::ItemRow {
            item_def_id: "torch".to_string(),
            quantity: 1,
            equip_slot: None,
            enchant: 0,
            cape_color: None,
            cape_texture: None,
        }]
    );
}

// F-015: session replacement (kick) must flush the departing session's
// inventory to the DB before a replacement login can load it. Otherwise the
// old async disconnect-save races the new session's load, letting a dropped
// item survive in the DB snapshot and be picked up again (duplication).
#[tokio::test]
async fn kick_flushes_dropped_inventory_before_replacement_load() {
    let auth = make_test_auth("f015_kick_flush");
    let account = auth.login_npc("npc_f015_account").unwrap();
    let attributes = attrs_with_cha(12);
    let record = create_test_character(&auth, &account, "Dupeknight");
    let char_id = record.id;

    // DB snapshot before the drop: one sword in the bag.
    auth.save_batch(
        &[],
        &[(
            char_id,
            vec![crate::auth::ItemRow {
                item_def_id: "worn_iron_sword".to_string(),
                quantity: 1,
                equip_slot: None,
                enchant: 0,
                cape_color: None,
                cape_texture: None,
            }],
        )],
        &[],
        &[],
        None,
    )
    .unwrap();

    let game_state = make_test_game_state("f015_kick_flush");

    // Session A enters with its account, player, and inventory registered.
    let a = pid("session_a");
    let (kick_tx, mut kick_rx) = tokio::sync::mpsc::unbounded_channel();
    let session_a = game_state
        .register_account_session(&account, kick_tx, &auth)
        .await;
    let mut player = make_player("session_a", 0.0, 0.0);
    player.name = record.name.clone();
    game_state.add_player(player).await;
    game_state
        .register_player_character(&a, char_id, record.xp, attributes, record.gold, None)
        .await;
    assert!(
        game_state
            .attach_player_to_account_session(&account, session_a, a)
            .await
    );
    game_state.load_player_inventory(&a, char_id, &auth).await;

    // A drops the sword (in-memory only; not yet persisted).
    let instance_id = game_state.get_player_inventory(&a).await.unwrap().bag[0].instance_id;
    game_state.drop_item(&a, instance_id).await;
    assert!(game_state
        .get_player_inventory(&a)
        .await
        .unwrap()
        .bag
        .is_empty());
    // The window F-015 raced: the DB still holds the pre-drop snapshot.
    assert_eq!(auth.load_inventory(char_id).unwrap().len(), 1);

    // A replacement login kicks A at the account level.
    let (replacement_tx, _replacement_rx) = tokio::sync::mpsc::unbounded_channel();
    game_state
        .register_account_session(&account, replacement_tx, &auth)
        .await;
    assert!(matches!(
        kick_rx.try_recv(),
        Ok(KickNotice {
            message: ServerMessage::Kicked { player_id, .. },
            ..
        }) if player_id == a
    ));

    // The kick flushed A's post-drop inventory and detached it, so a
    // replacement load reads zero swords (no dupe) instead of a stale one.
    assert_eq!(auth.load_inventory(char_id).unwrap().len(), 0);
    assert!(game_state.get_player_inventory(&a).await.is_none());
}

fn make_door_test_house() -> onlinerpg_shared::housing::HouseData {
    use crate::housing::test_fixtures::{house_at, room_at};
    let mut house = house_at(10.0, 10.0, vec![room_at(0, 0)]);
    house.rooms[0].wall_north[0].variant = onlinerpg_shared::housing::WallVariant::WithDoor;
    house
}

#[tokio::test]
async fn open_door_state_is_stamped_onto_served_house_data() {
    let game_state = make_test_game_state("door_state_stamp");
    let house = make_door_test_house();
    game_state.housing_io.write_house(&house).await.unwrap();

    // Door world position: origin + (segment 0 center, north edge) = (10.5, 10)
    let toggler = pid("toggler");
    game_state
        .add_player(make_player("toggler", 10.5, 10.5))
        .await;

    let toggled = game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    assert_eq!(toggled, Some(true));

    // A reconnecting client re-fetches the house from disk (is_open false there);
    // the stamp overlays the in-memory open state.
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(served[0].rooms[0].wall_north[0].is_open);

    // Closing the door clears the stamp.
    game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(!served[0].rooms[0].wall_north[0].is_open);

    // A house edit re-seeds door state from the file (closed there).
    game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    game_state.passability_add_house(&house).await;
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(!served[0].rooms[0].wall_north[0].is_open);
}

#[tokio::test]
async fn door_saved_open_is_served_open_and_first_toggle_closes_it() {
    let game_state = make_test_game_state("door_saved_open");
    let mut house = make_door_test_house();
    house.rooms[0].wall_north[0].is_open = true;
    game_state.housing_io.write_house(&house).await.unwrap();
    game_state.passability_add_house(&house).await;

    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(served[0].rooms[0].wall_north[0].is_open);

    let toggler = pid("toggler");
    game_state
        .add_player(make_player("toggler", 10.5, 10.5))
        .await;
    let toggled = game_state
        .toggle_door(&toggler, &house.id, 0, WallDirection::North, 0)
        .await;
    assert_eq!(toggled, Some(false));

    // The file still says open; a client fetching it must see the live state.
    let mut served = vec![house.clone()];
    game_state.apply_open_door_state(&mut served).await;
    assert!(!served[0].rooms[0].wall_north[0].is_open);
}

fn item_row(
    item_def_id: &str,
    quantity: u32,
    equip_slot: Option<&str>,
    enchant: i32,
) -> crate::auth::ItemRow {
    crate::auth::ItemRow {
        item_def_id: item_def_id.to_string(),
        quantity,
        equip_slot: equip_slot.map(|s| s.to_string()),
        enchant,
        cape_color: None,
        cape_texture: None,
    }
}

/// Persist `rows` as a character's saved inventory, then log a player in
/// against them — the DB shape a returning session actually loads.
async fn load_saved_inventory(
    name: &str,
    rows: Vec<crate::auth::ItemRow>,
) -> (GameState, PlayerId) {
    let auth = make_test_auth(name);
    let account = auth.login_npc(&format!("npc_{name}")).unwrap();
    let record = create_test_character(&auth, &account, "Loader");
    auth.save_batch(&[], &[(record.id, rows)], &[], &[], None)
        .unwrap();

    let game_state = make_test_game_state(name);
    let p = pid(name);
    game_state.add_player(make_player(name, 0.0, 0.0)).await;
    game_state
        .register_player_character(&p, record.id, 0, attrs_with_cha(12), 0, None)
        .await;
    game_state.load_player_inventory(&p, record.id, &auth).await;
    (game_state, p)
}

/// A 64-hex content hash, the only shape a worn `cape_texture` takes.
const TEXTURE_HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A dyed cape is only dyed while the colour and print survive the DB round
/// trip.
#[tokio::test]
async fn a_dyed_cape_loads_back_dyed() {
    let (game_state, p) = load_saved_inventory(
        "dyed_cape_reload",
        vec![crate::auth::ItemRow {
            item_def_id: "wool_cape".to_string(),
            quantity: 1,
            equip_slot: Some("back".to_string()),
            enchant: 0,
            cape_color: Some("#3355ff".to_string()),
            cape_texture: Some(TEXTURE_HASH.to_string()),
        }],
    )
    .await;

    let inv = game_state.get_player_inventory(&p).await.unwrap();
    let cape = inv.equipped.get(&EquipSlot::Back).unwrap();
    assert_eq!(cape.cape_color.as_deref(), Some("#3355ff"));
    assert_eq!(cape.cape_texture.as_deref(), Some(TEXTURE_HASH));
}

/// Bags saved before trading/pickup learned to stack hold one row per unit;
/// the login load must merge them (same def, same enchant) so old characters
/// heal on their next session.
#[tokio::test]
async fn login_load_merges_legacy_split_stacks() {
    let (game_state, p) = load_saved_inventory(
        "load_merges_stacks",
        vec![
            item_row("apple", 1, None, 0),
            item_row("apple", 1, None, 0),
            item_row("apple", 1, None, 1),
            item_row("torch", 1, None, 0),
            item_row("torch", 1, None, 0),
        ],
    )
    .await;

    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&p].bag;
    let apples: Vec<_> = bag
        .iter()
        .filter(|i| i.item_def_id == "apple" && i.enchant == 0)
        .collect();
    assert_eq!(apples.len(), 1, "split enchant-0 apples must merge on load");
    assert_eq!(apples[0].quantity, 2);
    assert_eq!(
        bag.iter()
            .filter(|i| i.item_def_id == "apple" && i.enchant == 1)
            .count(),
        1,
        "a different enchant must keep its own slot"
    );
    assert_eq!(
        bag.iter().filter(|i| i.item_def_id == "torch").count(),
        2,
        "non-stackables must not merge on load"
    );
}

/// Rows unfold (a non-stackable saved with quantity N), merge, or add nothing,
/// and every id comes from one reserved range — overrunning it would hand a
/// later item an id a live one already holds, and bag lookups are by id.
#[tokio::test]
async fn login_load_keeps_instance_ids_distinct_across_row_shapes() {
    let (game_state, p) = load_saved_inventory(
        "load_id_range",
        vec![
            item_row("iron_sword", 1, Some("main_hand"), 0),
            item_row("torch", 3, None, 0),
            item_row("apple", 1, None, 0),
            item_row("apple", 1, None, 0),
            item_row("bread", 0, None, 0),
        ],
    )
    .await;

    let ids: Vec<u64> = {
        let inventories = game_state.inventories.read().await;
        let inv = &inventories[&p];
        let torches: Vec<_> = inv
            .bag
            .iter()
            .filter(|i| i.item_def_id == "torch")
            .collect();
        assert_eq!(torches.len(), 3, "a quantity-3 non-stackable row unfolds");
        assert!(torches.iter().all(|t| t.quantity == 1));
        let apples: Vec<_> = inv
            .bag
            .iter()
            .filter(|i| i.item_def_id == "apple")
            .collect();
        assert_eq!(apples.len(), 1);
        assert_eq!(apples[0].quantity, 2);
        assert!(
            inv.bag.iter().all(|i| i.item_def_id != "bread"),
            "an empty row adds nothing"
        );
        assert!(inv.equipped.contains_key(&EquipSlot::MainHand));
        inv.bag
            .iter()
            .chain(inv.equipped.values())
            .map(|i| i.instance_id)
            .collect()
    };

    let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "no two live items share an id");
    let next = game_state.reserve_instance_ids(1).await;
    assert!(
        ids.iter().all(|id| *id < next),
        "the cursor stayed inside the reserved range"
    );
}

use super::*;

#[tokio::test]
async fn pickup_broadcasts_the_pickup_animation() {
    let game_state = make_test_game_state("pickup_anim_broadcast");
    game_state.add_player(make_player("picker", 0.0, 0.0)).await;
    game_state
        .add_player(make_player("watcher", 2.0, 0.0))
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("picker"), Default::default());
    }
    {
        let mut ground_items = game_state.ground_items.write().await;
        ground_items.insert(
            42,
            ServerGroundItem {
                item: GroundItem {
                    instance_id: 42,
                    item_def_id: "test_item".to_string(),
                    position: Position {
                        x: 0.5,
                        y: 0.0,
                        z: 0.0,
                    },
                    floor_level: 0,
                    quantity: 1,
                    enchant: 0,
                    dropped_by: None,
                    cape_color: None,
                    cape_texture: None,
                },
                dropped_at_ms: 0,
            },
        );
    }
    let mut watcher_rx = game_state.register_direct_channel(&pid("watcher")).await;
    let mut picker_rx = game_state.register_direct_channel(&pid("picker")).await;

    // Driven by PickupStarted at the clip's first frame, not by the pickup
    // itself — which lands a third of a clip later, at the grab moment.
    game_state.broadcast_pickup_animation(&pid("picker")).await;

    let mut saw_animation = false;
    for msg in drain(&mut watcher_rx) {
        if let ServerMessage::PlayerInteractionChanged {
            player_id,
            object_type,
        } = msg
        {
            assert_eq!(player_id, pid("picker"));
            assert_eq!(object_type.as_deref(), Some("pickup"));
            saw_animation = true;
        }
    }
    assert!(
        saw_animation,
        "nearby players must see the pickup animation"
    );

    // The picker already plays it locally, so it is excluded from the fan-out.
    for msg in drain(&mut picker_rx) {
        assert!(
            !matches!(msg, ServerMessage::PlayerInteractionChanged { .. }),
            "the picker must not receive its own pickup broadcast"
        );
    }

    // The pickup itself no longer carries the animation.
    game_state.pickup_item(&pid("picker"), 42).await;
    let after_pickup = drain(&mut watcher_rx);
    for msg in &after_pickup {
        assert!(
            !matches!(msg, ServerMessage::PlayerInteractionChanged { .. }),
            "pickup_item must not broadcast the animation a second time"
        );
    }
    // Who took it, so a bard knows its tip walked off.
    assert!(
        after_pickup.iter().any(|msg| {
            matches!(msg, ServerMessage::GroundItemRemoved { instance_id, picked_up_by }
                if *instance_id == 42 && picked_up_by.as_ref() == Some(&pid("picker")))
        }),
        "the removal must say who picked it up"
    );
}

/// The killing blow lands partway into the swing, so a monster's loot must not
/// exist before then — an item that exists is one any client can ask to pick
/// up, animation or no animation.
#[tokio::test(start_paused = true)]
async fn monster_loot_is_withheld_until_the_killing_blow_lands() {
    let game_state = make_test_game_state("monster_loot_impact_delay");
    game_state.add_player(make_player("killer", 0.0, 0.0)).await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("killer"), Default::default());
    }
    let mut killer_rx = game_state.register_direct_channel(&pid("killer")).await;

    let drop_position = Position {
        x: 0.5,
        y: 0.0,
        z: 0.0,
    };
    game_state.spawn_kill_loot_after_impact(
        Some(GroundItem {
            instance_id: 7,
            item_def_id: "test_item".to_string(),
            position: drop_position,
            floor_level: 0,
            quantity: 1,
            enchant: 0,
            dropped_by: None,
            cape_color: None,
            cape_texture: None,
        }),
        Vec::new(),
        drop_position,
        0,
        None,
    );

    // Advance virtual time so the withheld task has been polled to its timer.
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;

    // Nothing exists yet, so nobody — patched client or not — can reach it.
    assert!(game_state.ground_items.read().await.is_empty());
    game_state.pickup_item(&pid("killer"), 7).await;
    assert!(
        game_state
            .inventories
            .read()
            .await
            .get(&pid("killer"))
            .is_some_and(|inv| inv.bag.is_empty()),
        "loot must not be pickable before the blade lands"
    );
    for msg in drain(&mut killer_rx) {
        assert!(
            !matches!(msg, ServerMessage::GroundItemSpawned { .. }),
            "the drop must not be announced before the blade lands"
        );
    }

    // Past the drop's deadline. Virtual time, so this costs nothing.
    tokio::time::sleep(*super::super::combat::PLAYER_ATTACK_IMPACT_DELAY).await;

    assert!(
        game_state.ground_items.read().await.contains_key(&7),
        "the drop must exist once the blade has landed"
    );
    assert!(
        drain(&mut killer_rx).iter().any(|msg| {
            matches!(msg, ServerMessage::GroundItemSpawned { item }
                if item.instance_id == 7 && item.dropped_by.is_none())
        }),
        "the drop must be announced once the blade has landed"
    );
}

/// A hand-dropped item owes nobody an animation, so it lands at once — and
/// the spawn names who dropped it, which is how a busker knows a tip from
/// the rest of what lies about.
#[tokio::test]
async fn a_hand_dropped_item_spawns_immediately() {
    let game_state = make_test_game_state("hand_drop_is_immediate");
    game_state
        .add_player(make_player("dropper", 0.0, 0.0))
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv = PlayerInventory::default();
        inv.bag.push(bag_item(11, "test_item", 1));
        inventories.insert(pid("dropper"), inv);
    }

    let mut dropper_rx = game_state.register_direct_channel(&pid("dropper")).await;
    game_state.drop_item(&pid("dropper"), 11).await;

    assert!(game_state.ground_items.read().await.contains_key(&11));
    assert!(
        drain(&mut dropper_rx).iter().any(|msg| {
            matches!(msg, ServerMessage::GroundItemSpawned { item }
                if item.instance_id == 11 && item.dropped_by.as_ref() == Some(&pid("dropper")))
        }),
        "the spawn must say who dropped it"
    );
}

#[tokio::test]
async fn pickup_animation_is_not_sent_beyond_the_delivery_radius() {
    let game_state = make_test_game_state("pickup_anim_radius");
    game_state.add_player(make_player("picker", 0.0, 0.0)).await;
    let far = super::EVENT_DELIVERY_RADIUS + 10.0;
    game_state
        .add_player(make_player("distant", far, 0.0))
        .await;
    let mut distant_rx = game_state.register_direct_channel(&pid("distant")).await;

    game_state.broadcast_pickup_animation(&pid("picker")).await;

    for msg in drain(&mut distant_rx) {
        assert!(
            !matches!(msg, ServerMessage::PlayerInteractionChanged { .. }),
            "the crouch must not reach players outside the delivery radius"
        );
    }
}

#[tokio::test]
async fn picked_up_stackable_joins_the_existing_stack() {
    let game_state = make_test_game_state("pickup_stacks");
    game_state.add_player(make_player("picker", 0.0, 0.0)).await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(11, "apple", 1));
        inventories.insert(pid("picker"), inv);
    }
    {
        let mut ground_items = game_state.ground_items.write().await;
        ground_items.insert(
            42,
            ServerGroundItem {
                item: GroundItem {
                    instance_id: 42,
                    item_def_id: "apple".to_string(),
                    position: Position {
                        x: 0.5,
                        y: 0.0,
                        z: 0.0,
                    },
                    floor_level: 0,
                    quantity: 1,
                    enchant: 0,
                    dropped_by: None,
                    cape_color: None,
                    cape_texture: None,
                },
                dropped_at_ms: 0,
            },
        );
    }

    game_state.pickup_item(&pid("picker"), 42).await;

    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&pid("picker")].bag;
    assert_eq!(bag.len(), 1, "the picked-up apple must join the stack");
    assert_eq!(bag[0].quantity, 2);
}

/// Puts a pile of `quantity` apples at the picker's feet.
async fn place_apple_pile(game_state: &GameState, instance_id: u64, quantity: u32) {
    game_state.ground_items.write().await.insert(
        instance_id,
        ServerGroundItem {
            item: GroundItem {
                instance_id,
                item_def_id: "apple".to_string(),
                position: Position {
                    x: 0.5,
                    y: 0.0,
                    z: 0.0,
                },
                floor_level: 0,
                quantity,
                enchant: 0,
                dropped_by: None,
                cape_color: None,
                cape_texture: None,
            },
            dropped_at_ms: 0,
        },
    );
}

#[tokio::test]
async fn a_whole_pile_comes_along_in_one_pickup() {
    let game_state = make_test_game_state("pickup_whole_pile");
    game_state.add_player(make_player("picker", 0.0, 0.0)).await;
    game_state
        .inventories
        .write()
        .await
        .insert(pid("picker"), Default::default());
    place_apple_pile(&game_state, 42, 12).await;

    game_state.pickup_item(&pid("picker"), 42).await;

    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&pid("picker")].bag;
    assert_eq!(bag.len(), 1);
    assert_eq!(bag[0].quantity, 12, "one click takes the whole pile");
    drop(inventories);
    assert!(
        game_state.ground_items.read().await.is_empty(),
        "an emptied pile is gone"
    );
}

/// A pile heavier than the remaining headroom must not be stranded: the picker
/// takes what fits and the rest stays put.
#[tokio::test]
async fn pickup_takes_only_what_fits_and_leaves_the_rest() {
    let game_state = make_test_game_state("pickup_partial_by_weight");
    game_state.add_player(make_player("picker", 0.0, 0.0)).await;
    // Default carry limit is 150; 495 apples at 0.3 each leave room for 5 more.
    game_state.inventories.write().await.insert(
        pid("picker"),
        PlayerInventory {
            bag: vec![bag_item(11, "apple", 495)],
            ..Default::default()
        },
    );
    let mut picker_rx = game_state.register_direct_channel(&pid("picker")).await;
    place_apple_pile(&game_state, 42, 12).await;

    game_state.pickup_item(&pid("picker"), 42).await;

    let inventories = game_state.inventories.read().await;
    assert_eq!(inventories[&pid("picker")].bag[0].quantity, 500);
    drop(inventories);

    let ground_items = game_state.ground_items.read().await;
    assert_eq!(
        ground_items[&42].item.quantity, 7,
        "the units that didn't fit stay on the ground"
    );
    drop(ground_items);

    assert!(
        drain(&mut picker_rx).iter().any(|msg| {
            matches!(msg, ServerMessage::GroundItemQuantityChanged { instance_id, quantity, .. }
                if *instance_id == 42 && *quantity == 7)
        }),
        "nearby clients must learn the pile shrank rather than vanished"
    );
}

#[tokio::test]
async fn a_pile_that_does_not_fit_at_all_stays_whole() {
    let game_state = make_test_game_state("pickup_no_headroom");
    game_state.add_player(make_player("picker", 0.0, 0.0)).await;
    game_state.inventories.write().await.insert(
        pid("picker"),
        PlayerInventory {
            bag: vec![bag_item(11, "apple", 500)],
            ..Default::default()
        },
    );
    place_apple_pile(&game_state, 42, 3).await;

    game_state.pickup_item(&pid("picker"), 42).await;

    assert_eq!(
        game_state.ground_items.read().await[&42].item.quantity,
        3,
        "nothing was taken"
    );
    assert_eq!(
        game_state.inventories.read().await[&pid("picker")].bag[0].quantity,
        500
    );
}

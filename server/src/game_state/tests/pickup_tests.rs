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
                    enchant: 0,
                    durability: None,
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
    for msg in drain(&mut watcher_rx) {
        assert!(
            !matches!(msg, ServerMessage::PlayerInteractionChanged { .. }),
            "pickup_item must not broadcast the animation a second time"
        );
    }
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
            enchant: 0,
            durability: None,
        }),
        drop_position,
        0,
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
        drain(&mut killer_rx).iter().any(
            |msg| matches!(msg, ServerMessage::GroundItemSpawned { item } if item.instance_id == 7)
        ),
        "the drop must be announced once the blade has landed"
    );
}

/// A hand-dropped item owes nobody an animation, so it lands at once.
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

    game_state.drop_item(&pid("dropper"), 11).await;

    assert!(game_state.ground_items.read().await.contains_key(&11));
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

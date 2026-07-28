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

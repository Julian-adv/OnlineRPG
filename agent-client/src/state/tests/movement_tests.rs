use super::*;

/// A state with one of everything a `move` target can name, so the ladder
/// is exercised against a populated world rather than an empty one.
fn targetable_state() -> (SharedState, mpsc::Receiver<ClientMessage>) {
    let (mut s, rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);

    let mut karl = test_player(3.0, 0.0);
    karl.id = PlayerId::from(2);
    karl.name = "Karl".to_string();
    s.nearby_players.insert(karl.id, karl);

    let mut near = monster("m2_1");
    near.monster_type = "goblin".to_string();
    near.position = p(5.0, 0.0, 0.0);
    let mut far = monster("m2_7");
    far.monster_type = "goblin".to_string();
    far.position = p(12.0, 0.0, 0.0);
    let mut other = monster("m2_9");
    other.monster_type = "slime".to_string();
    other.position = p(4.0, 0.0, 0.0);
    for m in [near, far, other] {
        s.nearby_monsters.insert(m.id.clone(), m);
    }

    s.remember_ground_item(ground_item(6043, "torch", 2.0, 0.0, 0));
    // Named after a species on purpose: "goblin" must still resolve to the
    // monsters, not to this sword.
    s.remember_ground_item(ground_item(6044, "goblin_sword", 2.5, 0.0, 0));
    s.remember_ground_item(ground_item(6045, "iron_sword", 3.0, 0.0, 0));
    (s, rx)
}

/// The shape of the string picks the namespace before any name lookup
/// runs, so an id and a name never compete.
#[test]
fn a_move_target_resolves_by_shape_then_by_name() {
    let (s, _rx) = targetable_state();

    assert_eq!(
        s.resolve_move_target("m2_1"),
        Ok(MoveTarget::Monster {
            id: "m2_1".to_string()
        })
    );
    assert_eq!(
        s.resolve_move_target("6043"),
        Ok(MoveTarget::GroundItem {
            instance_id: 6043,
            name: "torch".to_string()
        })
    );
    assert_eq!(
        s.resolve_move_target("karl"),
        Ok(MoveTarget::Character {
            id: PlayerId::from(2),
            name: "Karl".to_string()
        })
    );
    assert_eq!(
        s.resolve_move_target("torch"),
        Ok(MoveTarget::GroundItem {
            instance_id: 6043,
            name: "torch".to_string()
        })
    );
}

/// A bare number reaches the players rung: arrival events teach numeric
/// character ids, and a prop id must never shadow a person.
#[test]
fn a_numeric_target_resolves_to_a_character() {
    let (s, _rx) = targetable_state();
    assert_eq!(
        s.resolve_move_target("2"),
        Ok(MoveTarget::Character {
            id: PlayerId::from(2),
            name: "Karl".to_string()
        })
    );
}

/// Move-by-name matches the way pickup matches: the display name from
/// items.json and loose spellings work, not just the exact def id.
#[test]
fn a_ground_item_display_name_resolves_like_pickup() {
    let (s, _rx) = targetable_state();
    assert_eq!(
        s.resolve_move_target("Iron Sword"),
        Ok(MoveTarget::GroundItem {
            instance_id: 6045,
            name: "iron_sword".to_string()
        })
    );
}

/// A species is not an id, and saying so with the matching ids is what
/// lets the LLM fix the target on its next turn.
#[test]
fn a_monster_species_is_refused_with_the_ids_that_would_work() {
    let (s, _rx) = targetable_state();

    let Err(MoveTargetError::SpeciesNotId {
        species,
        candidates,
    }) = s.resolve_move_target("goblin")
    else {
        panic!("expected a species rejection");
    };
    assert_eq!(species, "goblin");
    // Nearest first, and only the goblins.
    assert_eq!(
        candidates
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["m2_1", "m2_7"]
    );
}

/// An id that no longer names anything in sight is a dead target, not an
/// unknown word — the LLM copied it from a stale world state.
#[test]
fn a_vanished_monster_id_says_it_is_gone() {
    let (s, _rx) = targetable_state();
    assert_eq!(
        s.resolve_move_target("m2_4"),
        Err(MoveTargetError::MonsterGone {
            id: "m2_4".to_string()
        })
    );
}

/// Nothing matched, so the reply carries what would have.
#[test]
fn an_unknown_target_lists_what_is_addressable() {
    let (s, _rx) = targetable_state();
    let Err(MoveTargetError::Unknown { addressable, .. }) = s.resolve_move_target("the tavern")
    else {
        panic!("expected an unknown target");
    };
    assert!(addressable.contains(&"Karl".to_string()), "{addressable:?}");
    assert!(addressable.contains(&"m2_1".to_string()), "{addressable:?}");
}

/// Dungeon names come from the registry and survive whatever casing and
/// spacing the LLM read them back with.
#[test]
fn a_dungeon_name_resolves_however_it_is_written() {
    let (s, _rx) = test_state();
    s.world_cache.write().unwrap().register_dungeons();

    for asked in ["Old Crypt", "old crypt", "old_crypt", "OldCrypt"] {
        assert_eq!(
            s.resolve_move_target(asked),
            Ok(MoveTarget::Dungeon {
                id: "old_crypt".to_string(),
                name: "Old Crypt".to_string()
            }),
            "{asked}"
        );
    }
}

fn tip_hat(owner: PlayerId) -> onlinerpg_shared::tip_hat::TipHat {
    onlinerpg_shared::tip_hat::TipHat {
        id: 900,
        owner,
        owner_name: "Me".to_string(),
        position: p(0.0, 0.0, 2.0),
        rotation: 0.0,
        floor_level: 0,
    }
}

/// A schedule departure folds our stall and picks our hat up by using the
/// hat item; someone else's hat is not ours to touch.
#[tokio::test]
async fn packing_up_before_leaving_folds_our_stall_and_tip_hat() {
    let (mut s, mut rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.self_bag = vec![onlinerpg_shared::inventory::ItemInstance {
        instance_id: 41,
        item_def_id: "tip_hat".to_string(),
        quantity: 1,
        enchant: 0,
        cape_color: None,
        cape_texture: None,
    }];

    s.tip_hats.insert(900, tip_hat(PlayerId::from(2)));
    s.pack_up_placeables("test").await;
    assert!(rx.try_recv().is_err(), "another busker's hat is left alone");

    s.tip_hats.insert(900, tip_hat(PlayerId::from(1)));
    s.stalls.insert(
        77,
        onlinerpg_shared::stall::Stall {
            id: 77,
            owner: PlayerId::from(1),
            position: p(0.0, 0.0, 1.0),
            rotation: 0.0,
            floor_level: 0,
        },
    );
    assert!(s.own_tip_hat().is_some());
    s.pack_up_placeables("test").await;
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientMessage::ChatMessage { message }) if message == "/pack_stall"
    ));
    assert!(matches!(
        rx.try_recv(),
        Ok(ClientMessage::UseItem { instance_id: 41 })
    ));
}

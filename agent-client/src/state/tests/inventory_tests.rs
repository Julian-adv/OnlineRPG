use super::*;

/// A stack survives a sale one unit at a time, so the units left in it
/// have to stay sellable for the rest of the turn. A drop takes the lot.
#[test]
fn selling_off_a_stack_leaves_the_rest_of_it_reachable() {
    let (mut s, _rx) = test_state();
    s.self_bag = vec![onlinerpg_shared::inventory::ItemInstance {
        instance_id: 7,
        item_def_id: "healing_potion".to_string(),
        quantity: 3,
        enchant: 0,
    }];

    let mut spent: HashMap<u64, u32> = HashMap::new();
    for unit in 1..=3 {
        let copies = s
            .find_carried_bag_copies("healing_potion", &spent)
            .unwrap_or_else(|| panic!("unit {unit} of 3 should still be in the bag"));
        let CarriedBagCopies::InBag { copies, .. } = copies else {
            panic!("expected InBag");
        };
        assert_eq!(copies, vec![(7, 4 - unit)]);
        *spent.entry(7).or_default() += 1;
    }
    assert!(s
        .find_carried_bag_copies("healing_potion", &spent)
        .is_none());

    let dropped = HashMap::from([(7, u32::MAX)]);
    assert!(s
        .find_carried_bag_copies("healing_potion", &dropped)
        .is_none());
}

/// A stack fragmented across two separate bag entries (e.g. two
/// non-stackable pickups sharing an item_def_id, or a stack that never
/// merged) is gathered as one pool spanning both instances.
#[test]
fn fragmented_stacks_are_gathered_across_every_instance() {
    let (mut s, _rx) = test_state();
    s.self_bag = vec![
        onlinerpg_shared::inventory::ItemInstance {
            instance_id: 1,
            item_def_id: "old_boot".to_string(),
            quantity: 1,
            enchant: 0,
        },
        onlinerpg_shared::inventory::ItemInstance {
            instance_id: 2,
            item_def_id: "old_boot".to_string(),
            quantity: 1,
            enchant: 0,
        },
    ];

    let CarriedBagCopies::InBag { def_id, copies } = s
        .find_carried_bag_copies("old_boot", &HashMap::new())
        .unwrap()
    else {
        panic!("expected InBag");
    };
    assert_eq!(def_id, "old_boot");
    assert_eq!(copies, vec![(1, 1), (2, 1)]);
}

/// Worn-only items report `WornOnly`, not `None` — the caller needs to
/// tell "nothing by that name" apart from "you're wearing it".
#[test]
fn worn_only_item_is_not_a_bag_copy() {
    let (mut s, _rx) = test_state();
    s.self_equipped.insert(
        onlinerpg_shared::inventory::EquipSlot::MainHand,
        onlinerpg_shared::inventory::ItemInstance {
            instance_id: 9,
            item_def_id: "iron_sword".to_string(),
            quantity: 1,
            enchant: 0,
        },
    );

    let CarriedBagCopies::WornOnly { def_id } = s
        .find_carried_bag_copies("iron_sword", &HashMap::new())
        .unwrap()
    else {
        panic!("expected WornOnly");
    };
    assert_eq!(def_id, "iron_sword");
}

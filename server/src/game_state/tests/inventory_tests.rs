use super::*;
use crate::game_state::inventory::{stack_into_bag, BagInsert};

fn insert(
    bag: &mut Vec<ItemInstance>,
    stackable: bool,
    def: &str,
    enchant: i32,
    id: u64,
    qty: u32,
) -> u64 {
    stack_into_bag(
        bag,
        BagInsert {
            stackable,
            item_def_id: def,
            enchant,
            first_instance_id: id,
            quantity: qty,
            cape_color: None,
            cape_texture: None,
        },
    )
}

#[test]
fn stack_into_bag_merges_only_same_def_and_enchant() {
    let mut bag = Vec::new();
    insert(&mut bag, true, "apple", 0, 1, 1);
    insert(&mut bag, true, "apple", 0, 2, 1);
    insert(&mut bag, true, "apple", 1, 3, 1);
    insert(&mut bag, false, "torch", 0, 4, 1);
    insert(&mut bag, false, "torch", 0, 5, 1);

    assert_eq!(bag.len(), 4, "merge only the same-def same-enchant pair");
    assert_eq!(bag[0].item_def_id, "apple");
    assert_eq!(bag[0].quantity, 2);
    assert_eq!(bag[1].enchant, 1);
    assert_eq!(bag[1].quantity, 1);
}

/// A non-stackable line never collapses into one multi-unit slot, and the
/// returned count is what batch callers advance their reserved range by.
#[test]
fn stack_into_bag_unfolds_non_stackable_quantities_into_one_slot_each() {
    let mut bag = Vec::new();
    assert_eq!(insert(&mut bag, false, "torch", 0, 100, 3), 3);
    assert_eq!(bag.len(), 3);
    assert!(bag.iter().all(|i| i.quantity == 1));
    assert_eq!(
        bag.iter().map(|i| i.instance_id).collect::<Vec<_>>(),
        vec![100, 101, 102]
    );

    assert_eq!(
        insert(&mut bag, true, "apple", 0, 200, 4),
        1,
        "a new stack takes one id"
    );
    assert_eq!(
        insert(&mut bag, true, "apple", 0, 201, 4),
        0,
        "a merge takes none"
    );
    assert_eq!(
        insert(&mut bag, true, "apple", 0, 202, 0),
        0,
        "an empty line is a no-op"
    );
    let apples: Vec<_> = bag.iter().filter(|i| i.item_def_id == "apple").collect();
    assert_eq!(apples.len(), 1);
    assert_eq!(apples[0].quantity, 8);
}

/// give_item feeds /give, dungeon chest loot, and grilling — repeated grants
/// of a stackable must share one bag entry.
#[tokio::test]
async fn give_item_stacks_repeated_grants() {
    let game_state = make_test_game_state("give_item_stacks");
    game_state.add_player(make_player("eater", 0.0, 0.0)).await;
    {
        let mut inventories = game_state.inventories.write().await;
        inventories.insert(pid("eater"), Default::default());
    }

    assert!(game_state.give_item(&pid("eater"), "grilled_minnow").await);
    assert!(game_state.give_item(&pid("eater"), "grilled_minnow").await);
    assert!(game_state.give_item(&pid("eater"), "fishing_rod").await);
    assert!(game_state.give_item(&pid("eater"), "fishing_rod").await);

    let inventories = game_state.inventories.read().await;
    let bag = &inventories[&pid("eater")].bag;
    let fish: Vec<_> = bag
        .iter()
        .filter(|i| i.item_def_id == "grilled_minnow")
        .collect();
    assert_eq!(fish.len(), 1);
    assert_eq!(fish[0].quantity, 2);
    let rods: Vec<_> = bag
        .iter()
        .filter(|i| i.item_def_id == "fishing_rod")
        .collect();
    assert_eq!(rods.len(), 2, "non-stackables keep their own slots");
}

/// Dropping from a stack must shed exactly one unit — before the stacking fix
/// a whole entry was removed and every unit above the first was destroyed.
#[tokio::test]
async fn dropping_from_a_stack_sheds_one_unit() {
    let game_state = make_test_game_state("drop_stack_unit");
    game_state
        .add_player(make_player("dropper", 0.0, 0.0))
        .await;
    {
        let mut inventories = game_state.inventories.write().await;
        let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
        inv.bag.push(bag_item(11, "apple", 3));
        inventories.insert(pid("dropper"), inv);
    }

    game_state.drop_item(&pid("dropper"), 11).await;

    {
        let inventories = game_state.inventories.read().await;
        let bag = &inventories[&pid("dropper")].bag;
        assert_eq!(bag.len(), 1, "the stack must survive the drop");
        assert_eq!(bag[0].quantity, 2);
        assert_eq!(bag[0].instance_id, 11, "the stack keeps its id");
    }
    {
        let ground_items = game_state.ground_items.read().await;
        assert_eq!(ground_items.len(), 1, "exactly one unit hits the ground");
        let dropped = ground_items.values().next().unwrap();
        assert_eq!(dropped.item.item_def_id, "apple");
        assert_ne!(
            dropped.item.instance_id, 11,
            "the shed unit needs its own id"
        );
    }

    // Draining the stack unit by unit ends with an empty bag, nothing lost.
    game_state.drop_item(&pid("dropper"), 11).await;
    game_state.drop_item(&pid("dropper"), 11).await;
    assert!(game_state.inventories.read().await[&pid("dropper")]
        .bag
        .is_empty());
    assert_eq!(game_state.ground_items.read().await.len(), 3);
}

// --- Two-handed weapons (doc/COMBAT.md 원거리 전투) ---

/// A player carrying a bow and a shield in the bag, plus a direct channel for
/// the refusal message.
async fn setup_two_hand_wielder(game_state: &GameState) -> DirectRx {
    game_state
        .add_player(make_player("wielder", 0.0, 0.0))
        .await;
    let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
    inv.bag.push(bag_item(1, "bow", 1));
    inv.bag.push(bag_item(2, "wooden_shield", 1));
    game_state
        .inventories
        .write()
        .await
        .insert(pid("wielder"), inv);
    game_state.register_direct_channel(&pid("wielder")).await
}

#[tokio::test]
async fn equipping_a_two_hander_empties_the_off_hand() {
    let game_state = make_test_game_state("two_hand_clears_off_hand");
    let _rx = setup_two_hand_wielder(&game_state).await;

    game_state.equip_item(&pid("wielder"), 2).await;
    game_state.equip_item(&pid("wielder"), 1).await;

    let inventories = game_state.inventories.read().await;
    let inv = &inventories[&pid("wielder")];
    assert_eq!(
        inv.equipped_def_id(EquipSlot::MainHand).as_deref(),
        Some("bow")
    );
    assert!(
        !inv.equipped.contains_key(&EquipSlot::OffHand),
        "the bow must claim the shield's hand"
    );
    assert!(
        inv.bag.iter().any(|i| i.item_def_id == "wooden_shield"),
        "the displaced shield goes back to the bag, not nowhere"
    );
}

#[tokio::test]
async fn an_off_hand_equip_is_refused_under_a_two_hander() {
    let game_state = make_test_game_state("two_hand_blocks_off_hand");
    let _rx = setup_two_hand_wielder(&game_state).await;

    game_state.equip_item(&pid("wielder"), 1).await;
    game_state.equip_item(&pid("wielder"), 2).await;

    let inventories = game_state.inventories.read().await;
    let inv = &inventories[&pid("wielder")];
    assert!(
        !inv.equipped.contains_key(&EquipSlot::OffHand),
        "no off-hand item may join a two-hander"
    );
    assert!(inv.bag.iter().any(|i| i.item_def_id == "wooden_shield"));
}

/// A one-handed weapon leaves the off-hand alone, so the rule costs today's
/// sword-and-shield nothing.
#[tokio::test]
async fn a_one_handed_weapon_keeps_the_off_hand() {
    let game_state = make_test_game_state("one_hand_keeps_off_hand");
    game_state
        .add_player(make_player("wielder", 0.0, 0.0))
        .await;
    let mut inv: onlinerpg_shared::inventory::PlayerInventory = Default::default();
    inv.bag.push(bag_item(1, "iron_sword", 1));
    inv.bag.push(bag_item(2, "wooden_shield", 1));
    game_state
        .inventories
        .write()
        .await
        .insert(pid("wielder"), inv);

    game_state.equip_item(&pid("wielder"), 1).await;
    game_state.equip_item(&pid("wielder"), 2).await;

    let inventories = game_state.inventories.read().await;
    let inv = &inventories[&pid("wielder")];
    assert_eq!(
        inv.equipped_def_id(EquipSlot::OffHand).as_deref(),
        Some("wooden_shield")
    );
}

use super::*;

/// The shared wallet path behind coin catches and coin piles: credits
/// accumulate and each award notifies with both GoldUpdate and GoldGained.
#[tokio::test]
async fn award_copper_updates_the_wallet_and_notifies() {
    let game_state = make_test_game_state("fishing_wallet");
    let (id, mut rx) = make_angler(&game_state, "angler_paid").await;

    game_state.award_copper(&id, 25).await;
    game_state.award_copper(&id, 10).await;

    let msgs = drain(&mut rx);
    assert!(
        msgs.iter()
            .any(|m| matches!(m, ServerMessage::GoldGained { amount } if *amount == 25)),
        "first credit announced"
    );
    assert!(
        msgs.iter()
            .any(|m| matches!(m, ServerMessage::GoldUpdate { gold } if *gold == 35)),
        "the wallet total accumulates to 35"
    );
}

/// A coin pouch is bag cargo until opened: `use_item` rolls its dice,
/// pays the wallet, spends the pouch, and the combat log says how much.
#[tokio::test]
async fn opening_a_coin_pouch_pays_its_copper_and_spends_it() {
    let game_state = make_test_game_state("fishing_pouch_open");
    let (id, mut rx) = make_angler(&game_state, "angler_purse").await;

    game_state.award_item(&id, "sunken_coin_pouch").await;
    let inv = game_state.get_player_inventory(&id).await.unwrap();
    let instance_id = inv
        .bag
        .iter()
        .find(|i| i.item_def_id == "sunken_coin_pouch")
        .expect("the pouch lands in the bag sealed")
        .instance_id;
    drain(&mut rx);

    game_state.use_item(&id, instance_id).await;
    let msgs = drain(&mut rx);
    let paid = msgs
        .iter()
        .find_map(|m| match m {
            ServerMessage::GoldGained { amount } => Some(*amount),
            _ => None,
        })
        .expect("opening the pouch must pay copper");
    assert!((3..=24).contains(&paid), "3d8 pays 3-24, got {paid}");
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            ServerMessage::SystemMessage { message }
                if message.contains(&format!("{paid} copper"))
        )),
        "the combat log must say how much spilled out"
    );
    let inv = game_state.get_player_inventory(&id).await.unwrap();
    assert!(
        !inv.bag.iter().any(|i| i.item_def_id == "sunken_coin_pouch"),
        "an opened pouch is spent"
    );
}

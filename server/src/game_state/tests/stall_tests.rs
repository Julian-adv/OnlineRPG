// ---- Merchant stalls (/lay_stall, /pack_stall) -----------------------------

use super::*;
use onlinerpg_shared::character::CharacterClass;

#[tokio::test]
async fn only_a_merchant_lays_a_stall_and_only_one_at_a_time() {
    let game_state = make_test_game_state("stall_lay");
    let auth = make_test_auth("stall_lay");
    let knight_id = pid("knight");
    game_state
        .add_player(make_player("knight", 100.0, 50.0))
        .await;
    let merchant_id = pid("npc_wick");
    let mut merchant = make_player("npc_wick", 100.0, 50.0);
    merchant.class = CharacterClass::Merchant;
    merchant.is_official_npc = true;
    game_state.add_player(merchant).await;

    game_state
        .send_chat_message(&knight_id, "/lay_stall".to_string(), &auth)
        .await;
    assert!(
        game_state.stalls.read().await.is_empty(),
        "non-merchants are refused"
    );

    game_state
        .send_chat_message(&merchant_id, "/lay_stall".to_string(), &auth)
        .await;
    {
        let stalls = game_state.stalls.read().await;
        assert_eq!(stalls.len(), 1);
        assert_eq!(stalls.values().next().unwrap().owner, merchant_id);
    }

    game_state
        .send_chat_message(&merchant_id, "/lay_stall".to_string(), &auth)
        .await;
    assert_eq!(
        game_state.stalls.read().await.len(),
        1,
        "a second stall by the same owner is refused"
    );
}

#[tokio::test]
async fn pack_stall_and_logout_both_fold_the_table() {
    let game_state = make_test_game_state("stall_pack");
    let auth = make_test_auth("stall_pack");
    let merchant_id = pid("npc_wick");
    let mut merchant = make_player("npc_wick", 100.0, 50.0);
    merchant.class = CharacterClass::Merchant;
    game_state.add_player(merchant).await;

    game_state
        .send_chat_message(&merchant_id, "/pack_stall".to_string(), &auth)
        .await;
    assert!(game_state.stalls.read().await.is_empty());

    game_state
        .send_chat_message(&merchant_id, "/lay_stall".to_string(), &auth)
        .await;
    assert_eq!(game_state.stalls.read().await.len(), 1);
    game_state
        .send_chat_message(&merchant_id, "/pack_stall".to_string(), &auth)
        .await;
    assert!(
        game_state.stalls.read().await.is_empty(),
        "/pack_stall removes the stall"
    );

    game_state
        .send_chat_message(&merchant_id, "/lay_stall".to_string(), &auth)
        .await;
    assert_eq!(game_state.stalls.read().await.len(), 1);
    game_state.remove_player(&merchant_id).await;
    assert!(
        game_state.stalls.read().await.is_empty(),
        "logout packs the stall up"
    );
}

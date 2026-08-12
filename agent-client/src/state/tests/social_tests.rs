use super::*;

/// Chat history is a capped ring stamped with the game clock — the
/// short-term memory a stateless backend gets replayed each prompt.
#[test]
fn chat_history_stamps_and_caps() {
    let (mut s, _rx) = test_state();
    s.push_chat_history("[Chat] jake1: hello");
    assert_eq!(s.chat_history()[0], "[Chat] jake1: hello");

    s.game_hour = Some(20);
    s.game_minute = Some(26);
    for i in 0..40 {
        s.push_chat_history(&format!("[Chat] jake1: line {i}"));
    }
    assert_eq!(s.chat_history().len(), 30, "capped at MAX_CHAT_HISTORY");
    assert_eq!(s.chat_history()[0], "[20:26] [Chat] jake1: line 10");
    assert_eq!(s.chat_history()[29], "[20:26] [Chat] jake1: line 39");
}

/// Favor: nearby human players only, one step per call, clamped in
/// total. Crossing the threshold makes a player keepsake-worthy and
/// the world state shows the standing next to their name.
#[test]
fn favor_accumulates_and_gates_keepsakes() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);

    let mut jake = test_player(3.0, 0.0);
    jake.id = PlayerId::from(2);
    jake.name = "jake1".to_string();
    s.nearby_players.insert(jake.id, jake);

    let mut wick = test_player(4.0, 0.0);
    wick.id = PlayerId::from(3);
    wick.name = "Wick".to_string();
    wick.is_official_npc = true;
    s.nearby_players.insert(wick.id, wick);

    assert!(!s.apply_favor("Wick", 1), "NPCs never earn favor");
    assert!(!s.apply_favor("stranger", 1), "unknown names are dropped");
    assert!(
        s.apply_favor("jake1", 5),
        "an oversized delta still steps once"
    );
    assert_eq!(s.favor.get("jake1"), Some(&1));
    assert!(s.trade_worthy_players().is_empty());

    for _ in 0..10 {
        s.apply_favor("jake1", 1);
    }
    assert_eq!(s.favor.get("jake1"), Some(&FAVOR_MAX));
    assert_eq!(s.trade_worthy_players(), ["jake1"]);
    assert!(
        s.format_world_state().contains("jake1 (favor +5)"),
        "{}",
        s.format_world_state()
    );

    // Even a favored regular is not courted while their decline
    // cooldown runs — the keepsake section drops them too.
    s.push_event(ServerMessage::TradeDeclined {
        player_id: PlayerId::from(2),
        player_name: "jake1".to_string(),
    });
    assert!(s.trade_worthy_players().is_empty());
}

/// The wishlist pitch needs an audience with any favor at all, and a
/// waved-off trade window blocks pushes at that player for the
/// cooldown — dropping them from the audience despite their favor.
#[test]
fn a_declined_trade_offer_blocks_further_pushes() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);

    let mut jake = test_player(3.0, 0.0);
    jake.id = PlayerId::from(2);
    jake.name = "jake1".to_string();
    s.nearby_players.insert(jake.id, jake);

    assert!(!s.trade_offer_blocked(&PlayerId::from(2)));
    assert!(
        s.trade_worthy_players().is_empty(),
        "a stranger does not earn the shopping list"
    );
    s.apply_favor("jake1", 1);
    assert!(
        s.trade_worthy_players().is_empty(),
        "one kindness does not yet make a regular"
    );
    for _ in 0..2 {
        s.apply_favor("jake1", 1);
    }
    assert_eq!(
        s.trade_worthy_players(),
        ["jake1"],
        "favor at the trade threshold earns the pitch"
    );

    s.push_event(ServerMessage::TradeDeclined {
        player_id: PlayerId::from(2),
        player_name: "jake1".to_string(),
    });

    assert!(s.trade_offer_blocked(&PlayerId::from(2)));
    assert!(
        !s.trade_offer_blocked(&PlayerId::from(3)),
        "the block is per player, not global"
    );
    assert!(
        s.trade_worthy_players().is_empty(),
        "the only regular declined: the section vanishes"
    );
}

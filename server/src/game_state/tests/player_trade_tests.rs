// ---- Player-to-player trading (doc/TRADE.md) --------------------------------
//
// The risk here is unlike anywhere else in the game: a duplication bug mints
// items that never decay and spread across the server before anyone notices.
// These cover the paths that could mint or lose one.

use super::*;
use crate::auth::{AuthService, CharacterRecord};

/// Two registered, adjacent players backed by real DB characters, so a commit
/// can be asserted against what actually landed on disk.
struct TradePair {
    game_state: GameState,
    auth: AuthService,
    a: PlayerId,
    b: PlayerId,
    a_record: CharacterRecord,
    b_record: CharacterRecord,
    account: String,
}

async fn make_trade_pair(test_name: &str, a_gold: i64, b_gold: i64) -> TradePair {
    let auth = make_test_auth(test_name);
    let account = auth.login_npc(&format!("npc_{test_name}")).unwrap();
    let a_record = create_test_character(&auth, &account, "Aldis");
    let b_record = create_test_character(&auth, &account, "Bryn");

    let game_state = make_test_game_state(test_name);
    let (a, b) = (pid("Aldis"), pid("Bryn"));
    for (id, name, x, record, gold) in [
        ("Aldis", "Aldis", 10.0, &a_record, a_gold),
        ("Bryn", "Bryn", 11.0, &b_record, b_gold),
    ] {
        let mut player = make_player(id, x, 0.0);
        player.name = name.to_string();
        game_state.add_player(player).await;
        game_state
            .register_player_character(&pid(id), record.id, 0, attrs_with_cha(12), gold, None)
            .await;
        game_state
            .inventories
            .write()
            .await
            .insert(pid(id), PlayerInventory::default());
    }
    TradePair {
        game_state,
        auth,
        a,
        b,
        a_record,
        b_record,
        account,
    }
}

async fn give(game_state: &GameState, player_id: &PlayerId, item: ItemInstance) {
    game_state
        .inventories
        .write()
        .await
        .get_mut(player_id)
        .unwrap()
        .bag
        .push(item);
}

async fn open_session(pair: &TradePair) {
    pair.game_state.request_player_trade(&pair.a, "Bryn").await;
    pair.game_state
        .respond_player_trade(&pair.b, &pair.a, true)
        .await;
}

async fn complete_trade(pair: &TradePair) {
    let rev = revision(&pair.game_state, &pair.a).await;
    pair.game_state.lock_player_trade(&pair.a, rev).await;
    pair.game_state.lock_player_trade(&pair.b, rev).await;
    pair.game_state
        .confirm_player_trade(&pair.a, rev, &pair.auth)
        .await;
    pair.game_state
        .confirm_player_trade(&pair.b, rev, &pair.auth)
        .await;
}

async fn move_x(game_state: &GameState, player_id: &PlayerId, x: f32) {
    game_state
        .players
        .write()
        .await
        .get_mut(player_id)
        .unwrap()
        .position
        .x = x;
}

async fn revision(game_state: &GameState, player_id: &PlayerId) -> u32 {
    game_state
        .player_trades
        .read()
        .await
        .get(player_id)
        .expect("a live session")
        .revision
}

async fn bag_of(game_state: &GameState, player_id: &PlayerId) -> Vec<ItemInstance> {
    game_state.inventories.read().await[player_id].bag.clone()
}

async fn gold_of(game_state: &GameState, player_id: &PlayerId) -> i64 {
    game_state.player_gold.read().await[player_id]
}

fn slot(instance_id: u64, quantity: u32) -> onlinerpg_shared::messages::PlayerTradeSlot {
    onlinerpg_shared::messages::PlayerTradeSlot {
        instance_id,
        quantity,
    }
}

/// Both sides lock and confirm: items and coin change hands exactly once.
#[tokio::test]
async fn a_completed_trade_moves_items_and_coin_once() {
    let pair = make_trade_pair("trade_completes", 500, 0).await;
    give(&pair.game_state, &pair.a, bag_item(1, "healing_potion", 3)).await;
    give(&pair.game_state, &pair.b, bag_item(2, "dagger", 1)).await;
    open_session(&pair).await;

    pair.game_state
        .set_player_trade_offer(&pair.a, vec![slot(1, 2)], 100)
        .await;
    pair.game_state
        .set_player_trade_offer(&pair.b, vec![slot(2, 1)], 0)
        .await;
    complete_trade(&pair).await;

    let a_bag = bag_of(&pair.game_state, &pair.a).await;
    let b_bag = bag_of(&pair.game_state, &pair.b).await;
    // One potion stays behind, two crossed; the dagger came back the other way.
    assert_eq!(
        a_bag
            .iter()
            .find(|i| i.item_def_id == "healing_potion")
            .unwrap()
            .quantity,
        1
    );
    assert_eq!(
        a_bag.iter().filter(|i| i.item_def_id == "dagger").count(),
        1
    );
    assert_eq!(
        b_bag
            .iter()
            .find(|i| i.item_def_id == "healing_potion")
            .unwrap()
            .quantity,
        2
    );
    assert!(b_bag.iter().all(|i| i.item_def_id != "dagger"));
    assert_eq!(gold_of(&pair.game_state, &pair.a).await, 400);
    assert_eq!(gold_of(&pair.game_state, &pair.b).await, 100);

    // Durable before either client is told: the commit does not wait for the
    // 32-second dirty flush.
    let saved_a = pair.auth.load_inventory(pair.a_record.id).unwrap();
    let saved_b = pair.auth.load_inventory(pair.b_record.id).unwrap();
    assert_eq!(
        saved_a
            .iter()
            .find(|r| r.item_def_id == "healing_potion")
            .unwrap()
            .quantity,
        1
    );
    assert!(saved_b.iter().any(|r| r.item_def_id == "healing_potion"));
    let reloaded = pair
        .auth
        .list_characters_with_equipment(&pair.account)
        .unwrap()
        .into_iter()
        .map(|(record, _)| record)
        .find(|c| c.id == pair.a_record.id)
        .unwrap();
    assert_eq!(reloaded.gold, 400);

    // And the session is gone, so nothing stays reserved.
    assert!(pair
        .game_state
        .player_trades
        .read()
        .await
        .get(&pair.a)
        .is_none());
}

/// The last-second swap: changing an offer after locking moves the revision,
/// so a confirm quoting the old one is refused and nothing moves.
#[tokio::test]
async fn a_swap_after_locking_invalidates_the_confirmation() {
    let pair = make_trade_pair("trade_swap", 0, 0).await;
    give(&pair.game_state, &pair.a, bag_item(1, "iron_sword", 1)).await;
    give(&pair.game_state, &pair.a, bag_item(2, "worn_torch", 1)).await;
    open_session(&pair).await;

    pair.game_state
        .set_player_trade_offer(&pair.a, vec![slot(1, 1)], 0)
        .await;
    let seen = revision(&pair.game_state, &pair.b).await;
    pair.game_state.lock_player_trade(&pair.b, seen).await;

    // A pulls the sword back out. B's confirm still quotes what they saw.
    pair.game_state
        .set_player_trade_offer(&pair.a, vec![], 0)
        .await;
    pair.game_state
        .confirm_player_trade(&pair.b, seen, &pair.auth)
        .await;

    let session = pair.game_state.player_trades.read().await;
    let live = session
        .get(&pair.b)
        .expect("session survives a stale confirm");
    assert!(live.revision != seen, "the offer change moved the revision");
    drop(session);
    assert_eq!(bag_of(&pair.game_state, &pair.a).await.len(), 2);
    assert!(bag_of(&pair.game_state, &pair.b).await.is_empty());
}

/// Two players opening on each other at once must not deadlock, and must not
/// leave two overlapping sessions.
#[tokio::test]
async fn simultaneous_opposing_requests_settle_into_one_session() {
    let pair = make_trade_pair("trade_race", 0, 0).await;
    let (gs_a, gs_b) = (pair.game_state.clone(), pair.game_state.clone());
    let (a, b) = (pair.a, pair.b);

    let forward = tokio::spawn(async move { gs_a.request_player_trade(&a, "Bryn").await });
    let backward = tokio::spawn(async move { gs_b.request_player_trade(&b, "Aldis").await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        forward.await.unwrap();
        backward.await.unwrap();
    })
    .await
    .expect("opposing requests do not deadlock");

    pair.game_state
        .respond_player_trade(&pair.b, &pair.a, true)
        .await;
    pair.game_state
        .respond_player_trade(&pair.a, &pair.b, true)
        .await;

    let trades = pair.game_state.player_trades.read().await;
    assert_eq!(trades.sessions.len(), 1, "only one session survives");
}

/// Overweight at commit rolls the whole thing back — no partial transfer, the
/// shape that mints "where did half my stuff go" tickets.
#[tokio::test]
async fn an_overweight_receiver_aborts_the_whole_trade() {
    let pair = make_trade_pair("trade_overweight", 0, 0).await;
    // STR 12 → 180kg. One iron sword weighs far less, so pile on enough to
    // put the receiver over the line.
    let heavy = ItemInstance {
        instance_id: 1,
        item_def_id: "iron_sword".to_string(),
        quantity: 500,
        enchant: 0,
        cape_color: None,
    };
    give(&pair.game_state, &pair.a, heavy).await;
    open_session(&pair).await;

    pair.game_state
        .set_player_trade_offer(&pair.a, vec![slot(1, 500)], 0)
        .await;
    complete_trade(&pair).await;

    let a_bag = bag_of(&pair.game_state, &pair.a).await;
    assert_eq!(a_bag.len(), 1, "the giver keeps everything");
    assert_eq!(a_bag[0].quantity, 500);
    assert!(bag_of(&pair.game_state, &pair.b).await.is_empty());
    assert!(
        pair.game_state
            .player_trades
            .read()
            .await
            .get(&pair.a)
            .is_none(),
        "a failed commit ends the session rather than leaving it half-done"
    );
}

/// Starter gear carries `untradeable`, which `basePrice` alone never enforced
/// between players.
#[tokio::test]
async fn untradeable_starter_gear_cannot_be_offered() {
    let pair = make_trade_pair("trade_untradeable", 0, 0).await;
    give(&pair.game_state, &pair.a, bag_item(1, "worn_iron_sword", 1)).await;
    open_session(&pair).await;

    pair.game_state
        .set_player_trade_offer(&pair.a, vec![slot(1, 1)], 0)
        .await;

    let trades = pair.game_state.player_trades.read().await;
    let session = trades
        .get(&pair.a)
        .expect("session survives a refused offer");
    assert!(
        session.side(&pair.a).unwrap().items.is_empty(),
        "the refused item never reaches the table"
    );
}

/// The soft reservation: what is on the table cannot be sold, dropped, used or
/// equipped out from under the trade.
#[tokio::test]
async fn offered_items_are_reserved_against_other_actions() {
    let pair = make_trade_pair("trade_reserved", 0, 0).await;
    give(&pair.game_state, &pair.a, bag_item(1, "healing_potion", 2)).await;
    open_session(&pair).await;
    pair.game_state
        .set_player_trade_offer(&pair.a, vec![slot(1, 1)], 0)
        .await;

    assert_eq!(pair.game_state.trade_reserved_quantity(&pair.a, 1).await, 1);
    pair.game_state.drop_item(&pair.a, 1).await;
    pair.game_state.use_item(&pair.a, 1).await;

    let bag = bag_of(&pair.game_state, &pair.a).await;
    assert_eq!(bag.len(), 1);
    assert_eq!(bag[0].quantity, 2, "neither drop nor use touched the stack");
}

/// A disconnect ends the session, so nothing stays reserved for a player who
/// is no longer there.
#[tokio::test]
async fn a_disconnect_releases_the_table() {
    let pair = make_trade_pair("trade_disconnect", 0, 0).await;
    give(&pair.game_state, &pair.b, bag_item(1, "torch", 1)).await;
    open_session(&pair).await;
    pair.game_state
        .set_player_trade_offer(&pair.b, vec![slot(1, 1)], 0)
        .await;

    pair.game_state.remove_player(&pair.a).await;

    assert!(pair
        .game_state
        .player_trades
        .read()
        .await
        .sessions
        .is_empty());
    assert_eq!(pair.game_state.trade_reserved_quantity(&pair.b, 1).await, 0);
}

/// NPCs stay out: their trading has price bands, budgets and cooldowns, and a
/// free-form swap would route around all of it.
#[tokio::test]
async fn npcs_refuse_player_trades() {
    let pair = make_trade_pair("trade_npc", 0, 0).await;
    let mut npc = make_player("Rica", 10.5, 0.0);
    npc.name = "Rica".to_string();
    npc.is_official_npc = true;
    pair.game_state.add_player(npc).await;
    let mut rx = pair.game_state.register_direct_channel(&pair.a).await;

    pair.game_state.request_player_trade(&pair.a, "Rica").await;

    let refused = drain(&mut rx).into_iter().any(|msg| {
        matches!(
            msg,
            ServerMessage::PlayerTradeRequestResult {
                accepted: false,
                ..
            }
        )
    });
    assert!(refused, "the request is refused server-side");
    assert!(pair
        .game_state
        .player_trades
        .read()
        .await
        .sessions
        .is_empty());
}

/// The session leaves the table the moment the second confirm lands, so a
/// confirm that was already in flight has nothing to run the swap on again.
#[tokio::test]
async fn a_repeated_confirm_cannot_run_the_swap_twice() {
    let pair = make_trade_pair("trade_double_confirm", 300, 0).await;
    give(&pair.game_state, &pair.b, bag_item(1, "healing_potion", 10)).await;
    open_session(&pair).await;
    pair.game_state
        .set_player_trade_offer(&pair.a, vec![], 100)
        .await;
    pair.game_state
        .set_player_trade_offer(&pair.b, vec![slot(1, 3)], 0)
        .await;
    let rev = revision(&pair.game_state, &pair.a).await;
    complete_trade(&pair).await;

    pair.game_state
        .confirm_player_trade(&pair.a, rev, &pair.auth)
        .await;
    pair.game_state
        .confirm_player_trade(&pair.b, rev, &pair.auth)
        .await;

    assert_eq!(gold_of(&pair.game_state, &pair.a).await, 200);
    assert_eq!(gold_of(&pair.game_state, &pair.b).await, 100);
    let b_bag = bag_of(&pair.game_state, &pair.b).await;
    assert_eq!(b_bag[0].quantity, 7);
    let a_bag = bag_of(&pair.game_state, &pair.a).await;
    assert_eq!(a_bag.len(), 1);
    assert_eq!(a_bag[0].quantity, 3);
}

/// Slots naming the same stack merge into one entry, and a pair whose sum
/// wraps u32 is refused rather than passing as a small number.
#[tokio::test]
async fn duplicate_slots_merge_and_cannot_overflow() {
    let pair = make_trade_pair("trade_dup_slots", 0, 0).await;
    give(&pair.game_state, &pair.a, bag_item(1, "healing_potion", 10)).await;
    open_session(&pair).await;

    pair.game_state
        .set_player_trade_offer(&pair.a, vec![slot(1, 1), slot(1, 1)], 0)
        .await;
    {
        let trades = pair.game_state.player_trades.read().await;
        let items = &trades.get(&pair.a).unwrap().side(&pair.a).unwrap().items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].quantity, 2);
    }

    pair.game_state
        .set_player_trade_offer(&pair.a, vec![slot(1, 6), slot(1, u32::MAX - 3)], 0)
        .await;
    {
        let trades = pair.game_state.player_trades.read().await;
        let items = &trades.get(&pair.a).unwrap().side(&pair.a).unwrap().items;
        assert_eq!(
            items.len(),
            1,
            "the overflowing offer was refused, the last one stands"
        );
        assert_eq!(items[0].quantity, 2);
    }
    assert_eq!(bag_of(&pair.game_state, &pair.a).await[0].quantity, 10);
}

/// A stall trade is measured to the table: the owner may stand well behind
/// it, and a customer far from the table cannot open it.
#[tokio::test]
async fn stall_trades_measure_range_to_the_table() {
    let pair = make_trade_pair("trade_stall_range", 0, 0).await;
    let stall = onlinerpg_shared::stall::Stall {
        id: 77,
        owner: pair.b,
        position: Position {
            x: 13.0,
            y: 0.0,
            z: 0.0,
        },
        rotation: 0.0,
        floor_level: 0,
    };
    pair.game_state.stalls.write().await.insert(stall.id, stall);
    move_x(&pair.game_state, &pair.b, 21.0).await;

    move_x(&pair.game_state, &pair.a, 5.0).await;
    pair.game_state
        .request_player_trade_at_stall(&pair.a, 77)
        .await;
    assert!(
        pair.game_state
            .player_trades
            .read()
            .await
            .get(&pair.a)
            .is_none(),
        "8 m from the table does not open"
    );

    move_x(&pair.game_state, &pair.a, 10.0).await;
    pair.game_state
        .request_player_trade_at_stall(&pair.a, 77)
        .await;
    assert!(
        pair.game_state
            .player_trades
            .read()
            .await
            .get(&pair.a)
            .is_some(),
        "3 m from the table opens, even with the owner 11 m away"
    );
}

/// Drifting off the table ends a stall trade like a cancel would, and the
/// customer waits out the reopen cooldown either way.
#[tokio::test]
async fn walking_off_a_stall_trade_charges_the_reopen_cooldown() {
    let pair = make_trade_pair("trade_stall_cooldown", 0, 0).await;
    let stall = onlinerpg_shared::stall::Stall {
        id: 78,
        owner: pair.b,
        position: Position {
            x: 12.0,
            y: 0.0,
            z: 0.0,
        },
        rotation: 0.0,
        floor_level: 0,
    };
    pair.game_state.stalls.write().await.insert(stall.id, stall);
    pair.game_state
        .request_player_trade_at_stall(&pair.a, 78)
        .await;
    let rev = revision(&pair.game_state, &pair.a).await;

    move_x(&pair.game_state, &pair.a, 30.0).await;
    pair.game_state
        .confirm_player_trade(&pair.a, rev, &pair.auth)
        .await;
    assert!(pair
        .game_state
        .player_trades
        .read()
        .await
        .get(&pair.a)
        .is_none());

    move_x(&pair.game_state, &pair.a, 10.0).await;
    pair.game_state
        .request_player_trade_at_stall(&pair.a, 78)
        .await;
    assert!(
        pair.game_state
            .player_trades
            .read()
            .await
            .get(&pair.a)
            .is_none(),
        "the stall stays shut to the customer who walked off"
    );
}

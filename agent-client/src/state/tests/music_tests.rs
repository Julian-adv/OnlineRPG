use super::*;

/// The LLM will happily call for a new song halfway through the last one,
/// which restarts the music for everyone listening. The command is dropped
/// and the model is told why — on its next prompt, not by waking it here.
#[test]
fn a_second_tune_is_refused_while_the_first_still_plays() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.in_game = true;

    assert!(
        !s.refuses_play_command("/play_music"),
        "nothing playing yet"
    );

    s.push_event(ServerMessage::PlayerMusicStarted {
        player_id: PlayerId::from(1),
        track: "Twilight Fields".to_string(),
        elapsed_secs: 0.0,
    });

    assert!(s.refuses_play_command("/play_music creekside"));
    assert!(s.refuses_play_command("/play_music"));
    assert!(
        !s.refuses_play_command("Any requests?"),
        "ordinary talk goes through"
    );
    assert!(
        !s.refuses_play_command("/play_musical chairs"),
        "only the whole command word counts"
    );

    let events = s.drain_agent_events();
    assert_eq!(events.len(), 2, "one note per dropped command: {events:?}");
    assert!(events[0].contains("still playing"), "{events:?}");
}

/// A title the LLM invented never reaches the server: it would answer
/// "No such song" an hour before the idle prompt showed it, with the
/// square still waiting on a song the bard announced.
#[test]
fn a_song_the_bard_does_not_know_is_refused_before_it_is_sent() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.in_game = true;

    assert!(s.refuses_play_command("/play_music Ballad of the Missing Track"));
    assert_eq!(s.take_wake_urgency(), EventUrgency::Urgent);
    let events = s.drain_agent_events();
    assert!(
        events[0].contains("Ballad of the Missing Track") && events[0].contains("songbook"),
        "{events:?}"
    );

    // A second guess is still refused, but only at the routine cadence.
    assert!(s.refuses_play_command("/play_music Another Invention"));
    assert_eq!(s.take_wake_urgency(), EventUrgency::Routine);

    // Songbook titles pass, including one folded from a "(1)" variant.
    assert!(!s.refuses_play_command("/play_music Twilight Fields"));
    assert!(!s.refuses_play_command("/play_music Wanderer of the Old Fields"));
    assert!(!s.refuses_play_command("/play_music creekside"));
    assert!(!s.refuses_play_command("/play_music"), "the random pick");
}

/// A busker pauses between songs. The agent gets no invitation to play
/// until the quiet spell is over, and asking early is refused with what
/// is left of it.
#[test]
fn a_song_is_followed_by_a_quiet_spell_before_the_next() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.in_game = true;

    s.push_event(ServerMessage::PlayerMusicStarted {
        player_id: PlayerId::from(1),
        track: "Twilight Fields".to_string(),
        elapsed_secs: 0.0,
    });
    s.push_event(ServerMessage::PlayerInteractionChanged {
        player_id: PlayerId::from(1),
        object_type: None,
    });

    let rest_until = s.self_music_rest_until.expect("a rest was scheduled");
    let rest = rest_until.saturating_duration_since(std::time::Instant::now());
    assert!(
        rest.as_secs() >= MUSIC_REST_MIN_SECS - 1 && rest.as_secs() <= MUSIC_REST_MAX_SECS,
        "{rest:?}"
    );

    assert!(
        s.refuses_play_command("/play_music"),
        "no encore during the rest"
    );
    s.check_music_finished();
    assert!(
        s.self_music_rest_until.is_some(),
        "the square is still resting"
    );

    s.self_music_rest_until = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    s.check_music_finished();
    let events = s.drain_agent_events();
    assert!(
        events.last().is_some_and(|e| e.contains("another song")),
        "{events:?}"
    );
    assert!(!s.refuses_play_command("/play_music"), "the rest is over");

    // In bed on the night schedule: playing would drop the sleeping pose
    // and nothing would put it back until morning.
    s.push_event(ServerMessage::PlayerInteractionChanged {
        player_id: PlayerId::from(1),
        object_type: Some("bed".to_string()),
    });
    assert!(s.refuses_play_command("/play_music"));
    let events = s.drain_agent_events();
    assert!(
        events.last().is_some_and(|e| e.contains("bed")),
        "{events:?}"
    );
}

/// The agent hears a tune start and end. Its own performance has no audio
/// to end it, so the registry's length is the clock — without that an NPC
/// bard would strum the same song forever.
#[test]
fn a_tune_is_announced_at_both_ends_and_our_own_stops_itself() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.in_game = true;

    s.push_event(ServerMessage::PlayerMusicStarted {
        player_id: PlayerId::from(1),
        track: "Twilight Fields".to_string(),
        elapsed_secs: 0.0,
    });

    s.check_music_finished();
    assert!(
        s.drain_pending_commands().is_empty(),
        "the song is still playing"
    );

    if let Some(p) = s.self_performance.as_mut() {
        p.ends_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    }
    s.check_music_finished();
    assert!(matches!(
        s.drain_pending_commands().as_slice(),
        [ClientMessage::StopInteraction]
    ));

    // The server clears the interaction; that is what the LLM reads.
    s.push_event(ServerMessage::PlayerInteractionChanged {
        player_id: PlayerId::from(1),
        object_type: None,
    });
    let events = s.drain_agent_events();
    assert!(
        events
            .iter()
            .any(|e| e.contains("You finished \"Twilight Fields\"")),
        "{events:?}"
    );
}

/// The 2:00 schedule replaces the strum with the bed pose before the song's
/// clock runs out. The stale performance must expire without a
/// StopInteraction — that would stand the sleeper up for the night.
#[test]
fn a_song_expiring_in_bed_does_not_stand_the_sleeper_up() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.in_game = true;

    s.push_event(ServerMessage::PlayerMusicStarted {
        player_id: PlayerId::from(1),
        track: "Twilight Fields".to_string(),
        elapsed_secs: 0.0,
    });
    // The schedule sent InteractObject(bed) and adopted the pose on send.
    s.self_player.as_mut().unwrap().object_type = Some("bed".to_string());
    if let Some(p) = s.self_performance.as_mut() {
        p.ends_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
    }

    s.check_music_finished();
    assert!(
        s.drain_pending_commands().is_empty(),
        "the bed pose must survive the stale song"
    );
    assert!(s.self_performance.is_none());
}

/// Coins thrown at a busker's feet wait for the end of the song — walking
/// over mid-tune would abandon the performance — and then name who to
/// thank. Loot that no player dropped is not a tip.
#[test]
fn tips_left_during_a_song_are_announced_when_it_ends() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.plays_music = true;
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.in_game = true;
    let mut listener = test_player(1.0, 0.0);
    listener.id = PlayerId::from(2);
    listener.name = "Mira".to_string();
    s.nearby_players.insert(listener.id, listener);
    let tipper = PlayerId::from(2);

    // A tip before the first note of the day counts too: a busker is a
    // busker whether or not it happens to be playing right then.
    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(1, "old_boot", 1.0, 0.0, tipper),
    });
    let events = s.drain_agent_events();
    assert!(
        events
            .iter()
            .any(|e| e.contains("[Tip] Mira left old_boot")),
        "{events:?}"
    );

    s.push_event(ServerMessage::PlayerMusicStarted {
        player_id: PlayerId::from(1),
        track: "Twilight Fields".to_string(),
        elapsed_secs: 0.0,
    });
    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(2, "coin_pile", 1.0, 0.0, tipper),
    });
    // A tip thrown from too far off, a monster's loot, and our own drop.
    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(3, "small_sword", TIP_RADIUS + 2.0, 0.0, tipper),
    });
    s.push_event(ServerMessage::GroundItemSpawned {
        item: ground_item(4, "goblin_sword", 1.0, 0.0, 0),
    });
    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(5, "mandolin", 1.0, 0.0, PlayerId::from(1)),
    });
    assert_eq!(s.pending_tips.len(), 1, "{:?}", s.pending_tips);
    // Drops still get their [Sighted] line mid-song; only the [Tip]
    // thanks waits for the music to end.
    let mid_song = s.drain_agent_events();
    assert!(
        mid_song.iter().all(|e| !e.contains("[Tip]")),
        "the song comes before the thanks: {mid_song:?}"
    );
    assert!(
        mid_song.iter().all(|e| !e.contains("mandolin")),
        "our own drop must not be sighted: {mid_song:?}"
    );

    s.push_event(ServerMessage::PlayerInteractionChanged {
        player_id: PlayerId::from(1),
        object_type: None,
    });
    let events = s.drain_agent_events();
    assert!(
        events
            .last()
            .is_some_and(|e| e.contains("[Tip] Mira left coin_pile") && e.contains("[id 2]")),
        "{events:?}"
    );
    assert_eq!(s.take_wake_urgency(), EventUrgency::Routine);
    // Still on the ground, and still remembered as Mira's.
    assert!(
        s.format_world_state()
            .contains("Item on ground: coin_pile (1.0m away) [id 2], dropped by Mira"),
        "{}",
        s.format_world_state()
    );

    // Tipped again during the quiet spell: nothing to wait for now.
    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(6, "gold_ring", 1.0, 0.0, tipper),
    });
    let events = s.drain_agent_events();
    assert!(
        events.last().is_some_and(|e| e.contains("gold_ring")),
        "{events:?}"
    );

    // Once the schedule has put it to bed, a tip is not worth getting up.
    s.push_event(ServerMessage::PlayerInteractionChanged {
        player_id: PlayerId::from(1),
        object_type: Some("bed".to_string()),
    });
    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(7, "gold_ring", 1.0, 0.0, tipper),
    });
    let events = s.drain_agent_events();
    assert!(!events.iter().any(|e| e.contains("[Tip]")), "{events:?}");
}

/// Nobody tips a guard for standing there: a drop in front of an agent
/// that does not busk is ordinary loot, and stays out of its events.
#[test]
fn only_a_busker_reads_a_drop_as_a_tip() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.in_game = true;
    let mut passer_by = test_player(1.0, 0.0);
    passer_by.id = PlayerId::from(2);
    s.nearby_players.insert(passer_by.id, passer_by);

    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(1, "coin_pile", 1.0, 0.0, PlayerId::from(2)),
    });

    let events = s.drain_agent_events();
    assert!(!events.iter().any(|e| e.contains("[Tip]")), "{events:?}");
}

/// A tip someone else grabs first is not thanked for at the end of the
/// song — the bard would be pointing at bare ground. It hears who took
/// it instead, and only once the song is over.
#[test]
fn a_tip_taken_before_the_song_ends_is_forgotten() {
    let (mut s, _rx) = test_state();
    let me = test_player(0.0, 0.0);
    s.plays_music = true;
    s.self_player_id = Some(me.id);
    s.self_player = Some(me);
    s.in_game = true;
    let mut listener = test_player(1.0, 0.0);
    listener.id = PlayerId::from(2);
    s.nearby_players.insert(listener.id, listener);
    let mut thief = test_player(1.0, 1.0);
    thief.id = PlayerId::from(3);
    thief.name = "Bran".to_string();
    s.nearby_players.insert(thief.id, thief);

    s.push_event(ServerMessage::PlayerMusicStarted {
        player_id: PlayerId::from(1),
        track: "Twilight Fields".to_string(),
        elapsed_secs: 0.0,
    });
    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(1, "coin_pile", 1.0, 0.0, PlayerId::from(2)),
    });
    s.push_event(ServerMessage::GroundItemRemoved {
        instance_id: 1,
        picked_up_by: Some(PlayerId::from(3)),
    });
    assert_eq!(s.take_wake_urgency(), EventUrgency::Noise, "not mid-song");
    s.push_event(ServerMessage::PlayerInteractionChanged {
        player_id: PlayerId::from(1),
        object_type: None,
    });

    let events = s.drain_agent_events();
    assert!(!events.iter().any(|e| e.contains("[Tip]")), "{events:?}");
    assert!(
        events
            .iter()
            .any(|e| e.contains("[GroundItem] Bran picked up coin_pile")),
        "{events:?}"
    );

    // Between songs there is nothing to hold it back.
    s.push_event(ServerMessage::GroundItemSpawned {
        item: dropped_item(2, "gold_ring", 1.0, 0.0, PlayerId::from(2)),
    });
    s.take_wake_urgency();
    s.push_event(ServerMessage::GroundItemRemoved {
        instance_id: 2,
        picked_up_by: Some(PlayerId::from(3)),
    });
    assert_eq!(s.take_wake_urgency(), EventUrgency::Routine);
    assert!(s
        .drain_agent_events()
        .iter()
        .any(|e| e.contains("Bran picked up gold_ring")));
}

/// A bard joining with the starter sword in hand swaps to its workhorse
/// — the cheapest instrument that is not an offerable keepsake — so the
/// good mandolin stays in the bag where the keepsake offer can reach
/// it. A bard already holding the workhorse (or a non-bard) is left
/// alone.
#[test]
fn a_joining_bard_takes_up_the_worn_mandolin() {
    use onlinerpg_shared::inventory::{EquipSlot, ItemInstance, PlayerInventory};

    let item = |instance_id: u64, def: &str| ItemInstance {
        instance_id,
        item_def_id: def.to_string(),
        quantity: 1,
        enchant: 0,
        cape_color: None,
    };
    let mut inventory = PlayerInventory {
        bag: vec![item(1, "worn_mandolin"), item(2, "mandolin")],
        equipped: HashMap::from([(EquipSlot::MainHand, item(3, "worn_iron_sword"))]),
    };

    let (mut s, _rx) = test_state();
    s.plays_music = true;
    s.keepsake_ids = vec!["mandolin".to_string()];
    s.push_event(ServerMessage::InventoryState {
        inventory: inventory.clone(),
    });
    let equips: Vec<_> = s
        .drain_pending_commands()
        .into_iter()
        .filter(|c| matches!(c, ClientMessage::EquipItem { instance_id: 1 }))
        .collect();
    assert_eq!(equips.len(), 1, "swap to the worn workhorse, once");

    // With an instrument in hand, the next snapshot changes nothing.
    inventory.bag = vec![item(2, "mandolin"), item(4, "worn_iron_sword")];
    inventory.equipped = HashMap::from([(EquipSlot::MainHand, item(1, "worn_mandolin"))]);
    s.push_event(ServerMessage::InventoryState {
        inventory: inventory.clone(),
    });
    assert!(
        !s.drain_pending_commands()
            .iter()
            .any(|c| matches!(c, ClientMessage::EquipItem { .. })),
        "the workhorse in hand is left alone"
    );

    // Holding the good mandolin steps down to the worn one, freeing the
    // keepsake back into the bag.
    inventory.bag = vec![item(1, "worn_mandolin"), item(4, "worn_iron_sword")];
    inventory.equipped = HashMap::from([(EquipSlot::MainHand, item(2, "mandolin"))]);
    s.push_event(ServerMessage::InventoryState {
        inventory: inventory.clone(),
    });
    assert!(
        s.drain_pending_commands()
            .iter()
            .any(|c| matches!(c, ClientMessage::EquipItem { instance_id: 1 })),
        "the good mandolin in hand gives way to the workhorse"
    );

    // A non-bard keeps whatever it holds.
    let (mut guard, _rx2) = test_state();
    inventory.equipped = HashMap::from([(EquipSlot::MainHand, item(3, "worn_iron_sword"))]);
    guard.push_event(ServerMessage::InventoryState { inventory });
    assert!(
        !guard
            .drain_pending_commands()
            .iter()
            .any(|c| matches!(c, ClientMessage::EquipItem { .. })),
        "only buskers reach for an instrument"
    );
}

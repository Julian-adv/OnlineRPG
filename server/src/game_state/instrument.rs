use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::messages::{
    InstrumentNoteEvent, INSTRUMENT_BATCH_MS, INSTRUMENT_MAX_EVENTS_PER_BATCH,
    INSTRUMENT_NOTE_COUNT, MUSIC_EMOTE,
};
use std::sync::atomic::Ordering;

pub(super) const INSTRUMENT_AUDIBLE_RADIUS: f32 = 30.0;

pub(super) fn valid_instrument_batch(events: &[InstrumentNoteEvent]) -> bool {
    if events.is_empty()
        || events.len() > INSTRUMENT_MAX_EVENTS_PER_BATCH
        || events[0].offset_ms != 0
    {
        return false;
    }

    events
        .iter()
        .all(|event| event.note < INSTRUMENT_NOTE_COUNT && event.offset_ms < INSTRUMENT_BATCH_MS)
        && events
            .windows(2)
            .all(|pair| pair[0].offset_ms <= pair[1].offset_ms)
}

impl super::GameState {
    pub(crate) async fn start_live_instrument(&self, player_id: &PlayerId) {
        if !self.holds_instrument(player_id).await {
            self.send_system_message(player_id, "You need an instrument to perform.")
                .await;
            return;
        }

        let pose = {
            let players = self.players.read().await;
            players.get(player_id).and_then(|player| {
                (player.health > 0 && player.is_ready(Self::now_ms()))
                    .then_some((player.position, player.floor_level))
            })
        };
        let Some((position, floor_level)) = pose else {
            self.send_system_message(player_id, "You can't perform right now.")
                .await;
            return;
        };

        self.cancel_fishing_if_active(player_id).await;
        self.cancel_grill_if_active(player_id).await;
        // A queued walk would cancel the session on the next movement tick.
        self.movement_intents.write().await.remove(player_id);
        // Release the guard before calling out: `set_player_interaction` takes
        // the same lock and tokio's RwLock is not reentrant.
        {
            let mut live_players = self.live_instrument_players.write().await;
            if !live_players.insert(*player_id) {
                return;
            }
            self.live_instruments_active.fetch_add(1, Ordering::Relaxed);
        }
        self.music_performances.write().await.remove(player_id);
        self.set_player_interaction(player_id, Some(MUSIC_EMOTE.to_string()), None)
            .await;
        // Same circle as PlayerMusicStarted: a listener who was hearing the
        // old tune from 31 m must be told it ended.
        self.send_direct_message_to_players_within_position(
            &position,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::PlayerInstrumentStarted {
                player_id: *player_id,
            },
            None,
        )
        .await;
        // A cancel (hit, trade, move tick) landing between the insert and the
        // broadcasts above already sent its None; ours must trail it or the
        // client keeps the panel open with nothing behind it.
        if !self
            .live_instrument_players
            .read()
            .await
            .contains(player_id)
        {
            self.set_player_interaction(player_id, None, None).await;
        }
    }

    pub(crate) async fn play_live_instrument_notes(
        &self,
        player_id: &PlayerId,
        events: Vec<InstrumentNoteEvent>,
    ) {
        if !valid_instrument_batch(&events)
            || !self
                .live_instrument_players
                .read()
                .await
                .contains(player_id)
        {
            return;
        }
        if !self.holds_instrument(player_id).await {
            self.cancel_live_instrument_if_active(player_id).await;
            return;
        }

        let pose = {
            let players = self.players.read().await;
            players.get(player_id).and_then(|player| {
                (player.health > 0 && player.is_ready(Self::now_ms())).then_some((
                    player.position,
                    player.floor_level,
                    player.name.clone(),
                ))
            })
        };
        let Some((position, floor_level, performer_name)) = pose else {
            self.cancel_live_instrument_if_active(player_id).await;
            return;
        };

        let mut recipients = self
            .player_ids_within_position(&position, floor_level, INSTRUMENT_AUDIBLE_RADIUS)
            .await;
        // Same rule as chat: a blocked player's output does not reach you.
        {
            let blocked = self.blocked_names.read().await;
            if !blocked.is_empty() {
                recipients.retain(|id| {
                    !blocked
                        .get(id)
                        .is_some_and(|names| names.contains(&performer_name))
                });
            }
        }
        self.send_direct_message_to_players_except(
            &recipients,
            ServerMessage::PlayerInstrumentNotes {
                player_id: *player_id,
                position,
                floor_level,
                events,
            },
            Some(player_id),
        )
        .await;
    }

    /// Lock-free "nobody is performing" check for hot paths (every move packet).
    fn no_live_instrument_anywhere(&self) -> bool {
        self.live_instruments_active.load(Ordering::Relaxed) == 0
    }

    /// The one removal path, so the counter never drifts from the set.
    pub(super) async fn remove_live_instrument(&self, player_id: &PlayerId) -> bool {
        let removed = self.live_instrument_players.write().await.remove(player_id);
        if removed {
            self.live_instruments_active.fetch_sub(1, Ordering::Relaxed);
        }
        removed
    }

    pub(crate) async fn cancel_live_instrument_if_active(&self, player_id: &PlayerId) {
        if self.no_live_instrument_anywhere()
            || !self
                .live_instrument_players
                .read()
                .await
                .contains(player_id)
        {
            return;
        }
        if self.remove_live_instrument(player_id).await {
            self.set_player_interaction(player_id, None, None).await;
        }
    }

    /// After an item leaves the bag: the session ends only if the instrument
    /// went with it. Gear changes never touch it (`holds_instrument` scans
    /// bag and hands alike).
    pub(super) async fn abort_instrument_if_lost(&self, player_id: &PlayerId) {
        if self.no_live_instrument_anywhere() {
            return;
        }
        if !self
            .live_instrument_players
            .read()
            .await
            .contains(player_id)
        {
            return;
        }
        if !self.holds_instrument(player_id).await {
            self.cancel_live_instrument_if_active(player_id).await;
        }
    }
}

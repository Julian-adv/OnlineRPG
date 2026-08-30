use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::messages::{
    InstrumentNoteEvent, INSTRUMENT_BATCH_MS, INSTRUMENT_NOTE_COUNT, MUSIC_EMOTE,
};

pub(super) const INSTRUMENT_AUDIBLE_RADIUS: f32 = 30.0;
pub(super) const MAX_INSTRUMENT_EVENTS_PER_BATCH: usize = 64;

pub(super) fn valid_instrument_batch(events: &[InstrumentNoteEvent]) -> bool {
    if events.is_empty()
        || events.len() > MAX_INSTRUMENT_EVENTS_PER_BATCH
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
        // Release the guard before calling out: `set_player_interaction` takes
        // the same lock and tokio's RwLock is not reentrant.
        {
            let mut live_players = self.live_instrument_players.write().await;
            if !live_players.insert(*player_id) {
                return;
            }
        }
        self.music_performances.write().await.remove(player_id);
        self.set_player_interaction(player_id, Some(MUSIC_EMOTE.to_string()), None)
            .await;
        self.send_direct_message_to_players_within_position(
            &position,
            floor_level,
            INSTRUMENT_AUDIBLE_RADIUS,
            ServerMessage::PlayerInstrumentStarted {
                player_id: *player_id,
            },
            None,
        )
        .await;
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
                (player.health > 0 && player.is_ready(Self::now_ms()))
                    .then_some((player.position, player.floor_level))
            })
        };
        let Some((position, floor_level)) = pose else {
            self.cancel_live_instrument_if_active(player_id).await;
            return;
        };

        self.send_direct_message_to_players_within_position(
            &position,
            floor_level,
            INSTRUMENT_AUDIBLE_RADIUS,
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

    pub(crate) async fn cancel_live_instrument_if_active(&self, player_id: &PlayerId) {
        if self.live_instrument_players.write().await.remove(player_id) {
            self.set_player_interaction(player_id, None, None).await;
        }
    }
}

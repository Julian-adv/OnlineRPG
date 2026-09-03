use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::messages::{
    InstrumentNoteEvent, INSTRUMENT_BATCH_MS, INSTRUMENT_NOTE_COUNT, MUSIC_EMOTE,
};

pub(super) const INSTRUMENT_AUDIBLE_RADIUS: f32 = 30.0;
/// Hands top out near ten notes per 250 ms; slack beyond that only serves
/// clients flooding listeners, who build audio nodes per note received.
pub(super) const MAX_INSTRUMENT_EVENTS_PER_BATCH: usize = 16;

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

    pub(crate) async fn cancel_live_instrument_if_active(&self, player_id: &PlayerId) {
        if self.live_instrument_players.write().await.remove(player_id) {
            self.set_player_interaction(player_id, None, None).await;
        }
    }
}

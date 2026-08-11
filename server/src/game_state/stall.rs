//! Merchant stalls (`/lay_stall`, `/pack_stall`): the table of wares a
//! merchant spreads out in front of themselves. Free of any item like the
//! NPC campfire, one per owner, packed up on command or logout. Never
//! persisted — a stall exists only while its owner is online.

use onlinerpg_shared::character::CharacterClass;
use onlinerpg_shared::stall::Stall;
use onlinerpg_shared::{PlayerId, ServerMessage};

use super::GameState;

impl GameState {
    /// `/lay_stall` — merchants only; refused while one is already out.
    pub(super) async fn lay_stall(&self, player_id: &PlayerId) {
        let (is_merchant, rotation) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.class == CharacterClass::Merchant, p.rotation),
                None => return,
            }
        };
        if !is_merchant {
            self.send_system_message(player_id, "Only a merchant can lay out a stall.")
                .await;
            return;
        }
        {
            let stalls = self.stalls.read().await;
            if stalls.values().any(|s| s.owner == *player_id) {
                drop(stalls);
                self.send_system_message(player_id, "Your stall is already laid out.")
                    .await;
                return;
            }
        }
        let Some((placement, floor_level)) = self
            .outdoor_placement(
                player_id,
                "You can only lay out a stall outdoors",
                "You can't lay out a stall in water",
            )
            .await
        else {
            return;
        };
        let stall = Stall {
            id: self.next_instance_id().await,
            owner: *player_id,
            position: placement,
            rotation,
            floor_level,
        };
        self.stalls.write().await.insert(stall.id, stall.clone());
        self.send_direct_message_to_players_within_position(
            &placement,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::StallPlaced { stall },
            None,
        )
        .await;
        self.send_system_message(player_id, "You lay out your stall.")
            .await;
    }

    /// `/pack_stall` — fold the table back up.
    pub(super) async fn pack_stall(&self, player_id: &PlayerId) {
        if self.remove_player_stall(player_id).await {
            self.send_system_message(player_id, "You pack up your stall.")
                .await;
        } else {
            self.send_system_message(player_id, "You have no stall laid out.")
                .await;
        }
    }

    /// Remove `player_id`'s stall if any, announcing it to the area. Also the
    /// logout path, so it says nothing to the owner itself.
    pub(super) async fn remove_player_stall(&self, player_id: &PlayerId) -> bool {
        let removed = {
            let mut stalls = self.stalls.write().await;
            let id = stalls
                .values()
                .find(|s| s.owner == *player_id)
                .map(|s| s.id);
            id.and_then(|id| stalls.remove(&id))
        };
        let Some(stall) = removed else {
            return false;
        };
        self.send_direct_message_to_players_within_position(
            &stall.position,
            stall.floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::StallRemoved { stall_id: stall.id },
            None,
        )
        .await;
        true
    }
}

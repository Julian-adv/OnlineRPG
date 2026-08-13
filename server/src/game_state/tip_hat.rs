//! Tip hats: the hat a performer sets down with the `tip_hat` item so
//! listeners can drop coins in it. Keyed by owner, one each, never persisted.

use onlinerpg_shared::tip_hat::TipHat;
use onlinerpg_shared::{PlayerId, ServerMessage};

use super::GameState;

/// Two cells in front of the performer, so it stands between them and the
/// audience rather than under their feet.
const TIP_HAT_PLACEMENT_M: f32 = 2.0;

/// How close a tipper must stand to the hat. Roomier than the client's reach
/// so a step taken while the dialog is open doesn't void the tip.
const MAX_TIP_DISTANCE_M: f32 = 5.0;

impl GameState {
    /// Using the `tip_hat` item: set it down, or pick it back up when one is
    /// already out. The item itself is never consumed.
    pub(super) async fn toggle_tip_hat(&self, player_id: &PlayerId) {
        if self.remove_player_tip_hat(player_id).await {
            self.send_system_message(player_id, "You pick your tip hat back up.")
                .await;
            return;
        }
        if self
            .reject_if_defeated(player_id, "You can't set a tip hat down while defeated")
            .await
        {
            return;
        }
        let Some((placement, floor_level)) = self
            .outdoor_placement(
                player_id,
                TIP_HAT_PLACEMENT_M,
                "You can only set a tip hat down outdoors",
                "You can't set a tip hat down in water",
            )
            .await
        else {
            return;
        };
        let Some((owner_name, rotation)) = ({
            let players = self.players.read().await;
            players.get(player_id).map(|p| (p.name.clone(), p.rotation))
        }) else {
            return;
        };
        let tip_hat = TipHat {
            id: self.next_instance_id().await,
            owner: *player_id,
            owner_name,
            position: placement,
            rotation,
            floor_level,
        };
        self.tip_hats
            .write()
            .await
            .insert(*player_id, tip_hat.clone());
        self.send_direct_message_to_players_within_position(
            &placement,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::TipHatPlaced { tip_hat },
            None,
        )
        .await;
        self.send_system_message(player_id, "You set your tip hat down.")
            .await;
    }

    /// Remove `player_id`'s hat if any, announcing it to the area. Also the
    /// logout path, so it says nothing to the owner itself.
    pub(super) async fn remove_player_tip_hat(&self, player_id: &PlayerId) -> bool {
        let Some(tip_hat) = self.tip_hats.write().await.remove(player_id) else {
            return false;
        };
        self.send_direct_message_to_players_within_position(
            &tip_hat.position,
            tip_hat.floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::TipHatRemoved {
                tip_hat_id: tip_hat.id,
            },
            None,
        )
        .await;
        true
    }

    /// A hat its owner walked away from packs itself up. The mover's fanout
    /// decides `strayed` under the read lock it already holds.
    pub(super) async fn pack_up_strayed_tip_hat(&self, player_id: &PlayerId) {
        if self.remove_player_tip_hat(player_id).await {
            self.send_system_message(player_id, "You leave your tip hat behind and pick it up.")
                .await;
        }
    }

    /// Drop `amount` copper into someone else's hat.
    pub async fn tip_hat_tip(&self, player_id: &PlayerId, hat_id: u64, amount: i64) {
        if amount <= 0 {
            return;
        }
        let Some(hat) = ({
            let tip_hats = self.tip_hats.read().await;
            tip_hats.values().find(|h| h.id == hat_id).cloned()
        }) else {
            self.send_system_message(player_id, "That tip hat is gone.")
                .await;
            return;
        };
        if hat.owner == *player_id {
            self.send_system_message(player_id, "Tipping yourself pays nothing.")
                .await;
            return;
        }
        let Some((tipper_name, position, floor_level)) = ({
            let players = self.players.read().await;
            players
                .get(player_id)
                .map(|p| (p.name.clone(), p.position, p.floor_level))
        }) else {
            return;
        };
        if floor_level != hat.floor_level
            || hat.position.dist_xz_sq(&position) > MAX_TIP_DISTANCE_M * MAX_TIP_DISTANCE_M
        {
            self.send_system_message(player_id, "You're too far from that tip hat.")
                .await;
            return;
        }

        if !self.spend_copper(player_id, amount).await {
            self.send_system_message(player_id, "Not enough gold").await;
            return;
        }
        self.award_copper(&hat.owner, amount).await;

        let owner_name = hat.owner_name;
        self.send_system_message(
            player_id,
            format!("You drop {amount} copper into {owner_name}'s tip hat."),
        )
        .await;
        self.send_system_message(
            &hat.owner,
            format!("{tipper_name} drops {amount} copper into your tip hat."),
        )
        .await;
    }
}

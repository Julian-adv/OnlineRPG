use super::*;

impl SharedState {
    pub async fn send_command(&mut self, msg: ClientMessage) -> anyhow::Result<()> {
        let msg = match msg {
            ClientMessage::PlayerMove {
                position,
                rotation,
                append,
                sprinting,
                ..
            } => {
                // On the entrance stairs the wire floor is still 0 while the Y
                // already follows the ramp, so terrain height must not win there.
                let position = if self.self_floor_level == 0
                    && self.dungeon_ground_y(position.x, position.z, 0).is_none()
                {
                    self.snap_position_to_ground(position, "PlayerMove").await
                } else {
                    position
                };
                // Update local position immediately so subsequent reads don't use stale data
                if let Some(ref mut p) = self.self_player {
                    p.position = position;
                    p.rotation = rotation;
                }
                ClientMessage::PlayerMove {
                    position,
                    rotation,
                    floor_level: self.self_floor_level,
                    append,
                    sprinting,
                }
            }
            ClientMessage::RequestSpawnMonster {
                monster_type,
                position,
                rotation,
            } => ClientMessage::RequestSpawnMonster {
                monster_type,
                position: self
                    .snap_position_to_ground(position, "RequestSpawnMonster")
                    .await,
                rotation,
            },
            ClientMessage::MonsterMove {
                monster_id,
                position,
                rotation,
                state,
                target_position,
            } => {
                // A dungeon monster stands on its floor, not on the terrain
                // above it — snapping those to heightmap Y would haul the whole
                // floor's monsters up to the surface.
                let floor_level = self
                    .nearby_monsters
                    .get(&monster_id)
                    .map(|m| m.floor_level)
                    .unwrap_or(0);
                let (position, target_position) = if floor_level < 0 {
                    let floor = passability_floor_for_level(floor_level);
                    (
                        self.on_dungeon_floor(position, floor),
                        self.on_dungeon_floor(target_position, floor),
                    )
                } else {
                    // position and target_position are independent coordinates, so
                    // sample both terrain heights concurrently rather than serially.
                    tokio::join!(
                        self.snap_position_to_ground(position, "MonsterMove"),
                        self.snap_position_to_ground(target_position, "MonsterMove target"),
                    )
                };
                // The server skips echoing our own monster moves back;
                // mirror them locally or owned monsters freeze at spawn.
                self.apply_monster_pose(&monster_id, position, rotation, state);
                ClientMessage::MonsterMove {
                    monster_id,
                    position,
                    rotation,
                    state,
                    target_position,
                }
            }
            other => other,
        };
        self.cmd_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("Command channel closed: {e}"))
    }

    /// Drain pending commands (from monster AI reactions, spawn requests, etc.)
    pub fn drain_pending_commands(&mut self) -> Vec<ClientMessage> {
        std::mem::take(&mut self.pending_commands)
    }
}

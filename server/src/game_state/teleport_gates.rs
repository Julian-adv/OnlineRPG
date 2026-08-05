use onlinerpg_shared::dungeon::{cell_center, generate_dungeon_for, GRID};
use onlinerpg_shared::messages::TeleportGateDestination;
use onlinerpg_shared::teleport::{
    teleport_gate, teleport_gate_config, teleport_gate_distance_m, teleport_gate_fare,
    teleport_gates, TeleportGateDef,
};
use onlinerpg_shared::{
    shortest_world_delta_x, Position, WORLD_MAX_X, WORLD_MAX_Z, WORLD_MIN_X, WORLD_MIN_Z,
};
use rand::Rng;
use tracing::info;

use crate::types::{PlayerId, ServerMessage};

struct GateTravelOutcome {
    requested_town: String,
    arrival_description: String,
    fare: i64,
    misfired: bool,
}

struct GateArrival {
    position: Position,
    rotation: f32,
    floor_level: i8,
    description: String,
    misfired: bool,
}

fn mix_seed(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn random_fraction(seed: u64) -> f32 {
    let mantissa = (mix_seed(seed) >> 40) as u32;
    mantissa as f32 / (1_u32 << 24) as f32
}

fn random_between(seed: u64, min: f32, max: f32) -> f32 {
    min + random_fraction(seed) * (max - min)
}

impl super::GameState {
    fn dungeon_misfire_arrival(&self, seed: u64) -> Option<GateArrival> {
        let entrances: Vec<_> = self.dungeon_defs.all().collect();
        if entrances.is_empty() {
            return None;
        }
        let entrance = entrances.get(mix_seed(seed) as usize % entrances.len())?;
        let layouts = generate_dungeon_for(&entrance.id);
        if layouts.is_empty() {
            return None;
        }
        let layout = layouts.get(mix_seed(seed ^ 0xD06E) as usize % layouts.len())?;
        let cells: Vec<_> = layout
            .carved
            .iter()
            .enumerate()
            .filter_map(|(index, carved)| {
                if !*carved {
                    return None;
                }
                let cell = (
                    (index % GRID as usize) as i32,
                    (index / GRID as usize) as i32,
                );
                let occupied = layout.props.iter().any(|prop| (prop.x, prop.z) == cell)
                    || layout.spawns.iter().any(|spawn| (spawn.x, spawn.z) == cell)
                    || layout.chest == Some(cell);
                (!occupied).then_some(cell)
            })
            .collect();
        if cells.is_empty() {
            return None;
        }
        let cell = cells.get(mix_seed(seed ^ 0xCE11) as usize % cells.len())?;
        Some(GateArrival {
            position: cell_center(&entrance.position(), layout.depth, *cell),
            rotation: random_fraction(seed ^ 0xFACE) * std::f32::consts::TAU,
            floor_level: -(layout.depth as i8),
            description: format!("{} dungeon (depth {})", entrance.name, layout.depth),
            misfired: true,
        })
    }

    async fn surface_misfire_arrival(&self, seed: u64) -> Result<GateArrival, String> {
        let x = random_between(seed ^ 0x58, WORLD_MIN_X, WORLD_MAX_X);
        let z = random_between(seed ^ 0x5A, WORLD_MIN_Z, WORLD_MAX_Z);
        let terrain_y = self
            .height_sampler
            .sample_height(x, z)
            .await
            .map_err(|_| "The gate cannot find a wild destination".to_string())?;
        let water_y = self
            .water_sampler
            .sample_surface(x, z)
            .await
            .unwrap_or(terrain_y);
        let is_water = water_y > terrain_y + 0.4;
        Ok(GateArrival {
            position: Position {
                x,
                y: if is_water { water_y } else { terrain_y },
                z,
            },
            rotation: random_fraction(seed ^ 0xBEEF) * std::f32::consts::TAU,
            floor_level: 0,
            description: format!(
                "{} at ({x:.0}, {z:.0})",
                if is_water {
                    "open water"
                } else {
                    "remote wilderness"
                }
            ),
            misfired: true,
        })
    }

    async fn resolve_gate_arrival(
        &self,
        requested: &TeleportGateDef,
        misfire_roll_bps: u16,
        wild_kind_roll: u8,
        wild_seed: u64,
    ) -> Result<GateArrival, String> {
        let config = teleport_gate_config();
        if misfire_roll_bps < config.misfire_chance_bps {
            if wild_kind_roll < config.dungeon_misfire_percent {
                if let Some(arrival) = self.dungeon_misfire_arrival(wild_seed) {
                    return Ok(arrival);
                }
            }
            return self.surface_misfire_arrival(wild_seed).await;
        }

        let (x, z) = requested.arrival_xz();
        let y = self
            .height_sampler
            .sample_height(x, z)
            .await
            .map_err(|_| "The destination gate is temporarily unavailable".to_string())?;
        Ok(GateArrival {
            position: Position { x, y, z },
            rotation: requested.rotation + std::f32::consts::PI,
            floor_level: 0,
            description: requested.name.clone(),
            misfired: false,
        })
    }

    async fn send_teleport_gate_error(&self, player_id: &PlayerId, message: impl Into<String>) {
        self.send_direct_message(
            player_id,
            ServerMessage::TeleportGateError {
                message: message.into(),
            },
        )
        .await;
    }

    async fn validate_gate_access(
        &self,
        player_id: &PlayerId,
        gate: &TeleportGateDef,
    ) -> Result<(), &'static str> {
        let players = self.players.read().await;
        let player = players.get(player_id).ok_or("Player not found")?;
        if player.health == 0 {
            return Err("You cannot use a town gate while dead");
        }
        if player.floor_level != 0 {
            return Err("Town gates can only be used from the surface");
        }
        if Self::in_combat(player) {
            return Err("You cannot use a town gate while in combat");
        }
        let dx = shortest_world_delta_x(player.position.x, gate.x);
        let dz = gate.z - player.position.z;
        let range = teleport_gate_config().interaction_range_m;
        if dx * dx + dz * dz > range * range {
            return Err("Move closer to the town gate");
        }
        Ok(())
    }

    pub async fn open_teleport_gate(&self, player_id: &PlayerId, gate_id: &str) {
        let Some(gate) = teleport_gate(gate_id) else {
            return self
                .send_teleport_gate_error(player_id, "Unknown town gate")
                .await;
        };
        if let Err(message) = self.validate_gate_access(player_id, gate).await {
            return self.send_teleport_gate_error(player_id, message).await;
        }

        let mut destinations: Vec<_> = teleport_gates()
            .iter()
            .filter(|destination| destination.id != gate.id)
            .map(|destination| TeleportGateDestination {
                gate_id: destination.id.clone(),
                town_name: destination.name.clone(),
                distance_m: teleport_gate_distance_m(gate, destination).round() as u32,
                fare: teleport_gate_fare(gate, destination),
            })
            .collect();
        destinations.sort_by(|a, b| {
            a.fare
                .cmp(&b.fare)
                .then_with(|| a.town_name.cmp(&b.town_name))
        });
        self.send_direct_message(
            player_id,
            ServerMessage::TeleportGateState {
                gate_id: gate.id.clone(),
                town_name: gate.name.clone(),
                destinations,
                misfire_chance_bps: teleport_gate_config().misfire_chance_bps,
            },
        )
        .await;
    }

    pub async fn use_teleport_gate(
        &self,
        player_id: &PlayerId,
        gate_id: &str,
        destination_gate_id: &str,
    ) {
        let started = self.gate_traveling.lock().await.insert(*player_id);
        if !started {
            return self
                .send_teleport_gate_error(player_id, "A gate journey is already in progress")
                .await;
        }
        let (roll, wild_kind_roll, wild_seed) = {
            let mut rng = rand::thread_rng();
            (
                rng.gen_range(0..10_000),
                rng.gen_range(0..100),
                rng.gen::<u64>(),
            )
        };
        let result = self
            .use_teleport_gate_inner(
                player_id,
                gate_id,
                destination_gate_id,
                roll,
                wild_kind_roll,
                wild_seed,
            )
            .await;
        self.gate_traveling.lock().await.remove(player_id);

        match result {
            Ok(outcome) => {
                self.send_direct_message(
                    player_id,
                    ServerMessage::TeleportGateTravelled {
                        requested_town: outcome.requested_town,
                        arrival_description: outcome.arrival_description,
                        fare: outcome.fare,
                        misfired: outcome.misfired,
                    },
                )
                .await;
            }
            Err(message) => self.send_teleport_gate_error(player_id, message).await,
        }
    }

    async fn use_teleport_gate_inner(
        &self,
        player_id: &PlayerId,
        gate_id: &str,
        destination_gate_id: &str,
        misfire_roll_bps: u16,
        wild_kind_roll: u8,
        wild_seed: u64,
    ) -> Result<GateTravelOutcome, String> {
        let source = teleport_gate(gate_id).ok_or("Unknown town gate")?;
        let requested = teleport_gate(destination_gate_id).ok_or("Unknown destination gate")?;
        if source.id == requested.id {
            return Err("Choose another town".to_string());
        }
        self.validate_gate_access(player_id, source)
            .await
            .map_err(str::to_string)?;

        let fare = teleport_gate_fare(source, requested);
        let arrival = self
            .resolve_gate_arrival(requested, misfire_roll_bps, wild_kind_roll, wild_seed)
            .await?;

        self.validate_gate_access(player_id, source)
            .await
            .map_err(str::to_string)?;
        let remaining_gold = {
            let mut gold = self.player_gold.write().await;
            let balance = gold
                .get_mut(player_id)
                .ok_or_else(|| "Your wallet is unavailable".to_string())?;
            if *balance < fare {
                return Err("Not enough gold for that journey".to_string());
            }
            *balance -= fare;
            *balance
        };

        self.mark_dirty(player_id).await;
        self.send_direct_message(
            player_id,
            ServerMessage::GoldUpdate {
                gold: remaining_gold,
            },
        )
        .await;
        self.teleport_player(
            player_id,
            arrival.position,
            arrival.rotation,
            arrival.floor_level,
        )
        .await;
        info!(
            player = ?player_id,
            source = %source.id,
            requested = %requested.id,
            arrival = %arrival.description,
            floor_level = arrival.floor_level,
            fare,
            misfired = arrival.misfired,
            "town gate travel"
        );
        Ok(GateTravelOutcome {
            requested_town: requested.name.clone(),
            arrival_description: arrival.description,
            fare,
            misfired: arrival.misfired,
        })
    }

    #[cfg(test)]
    pub(super) async fn use_teleport_gate_for_test(
        &self,
        player_id: &PlayerId,
        gate_id: &str,
        destination_gate_id: &str,
        misfire_roll_bps: u16,
        wild_kind_roll: u8,
        wild_seed: u64,
    ) -> Result<(String, bool, i64), String> {
        self.use_teleport_gate_inner(
            player_id,
            gate_id,
            destination_gate_id,
            misfire_roll_bps,
            wild_kind_roll,
            wild_seed,
        )
        .await
        .map(|outcome| (outcome.arrival_description, outcome.misfired, outcome.fare))
    }
}

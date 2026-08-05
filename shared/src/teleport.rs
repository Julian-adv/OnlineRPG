use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::{shortest_world_delta_x, Position};

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeleportGateConfig {
    pub id: String,
    pub interaction_range_m: f32,
    pub arrival_offset_m: f32,
    pub base_fare_copper: i64,
    pub fare_per_km_copper: i64,
    pub misfire_chance_bps: u16,
    pub dungeon_misfire_percent: u8,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TeleportGateDef {
    pub id: String,
    pub name: String,
    pub x: f32,
    pub z: f32,
    pub rotation: f32,
}

impl TeleportGateDef {
    pub fn position(&self, y: f32) -> Position {
        Position {
            x: self.x,
            y,
            z: self.z,
        }
    }

    pub fn arrival_xz(&self) -> (f32, f32) {
        let offset = teleport_gate_config().arrival_offset_m;
        (
            self.x + self.rotation.sin() * offset,
            self.z + self.rotation.cos() * offset,
        )
    }
}

static TELEPORT_GATE_CONFIG: LazyLock<TeleportGateConfig> = LazyLock::new(|| {
    let mut configs: HashMap<String, TeleportGateConfig> =
        serde_json::from_str(include_str!("../../data/teleport_gate_config.json"))
            .expect("Failed to parse teleport_gate_config.json");
    let config = configs
        .remove("town_network")
        .expect("teleport gate config needs town_network");
    assert_eq!(config.id, "town_network");
    assert!(config.interaction_range_m > 0.0);
    assert!(config.arrival_offset_m > 0.0);
    assert!(config.base_fare_copper >= 0);
    assert!(config.fare_per_km_copper > 0);
    assert!(config.misfire_chance_bps <= 10_000);
    assert!((1..=100).contains(&config.dungeon_misfire_percent));
    config
});

static TELEPORT_GATES: LazyLock<Vec<TeleportGateDef>> = LazyLock::new(|| {
    let by_id: HashMap<String, TeleportGateDef> =
        serde_json::from_str(include_str!("../../data/teleport_gates.json"))
            .expect("Failed to parse teleport_gates.json");
    let mut gates: Vec<_> = by_id
        .into_iter()
        .map(|(key, gate)| {
            assert_eq!(key, gate.id, "teleport gate key and id must match");
            gate
        })
        .collect();
    gates.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        gates.len() >= 2,
        "teleport gate network needs at least two towns"
    );
    for gate in &gates {
        assert!(!gate.id.is_empty(), "teleport gate id must not be empty");
        assert!(
            !gate.name.is_empty(),
            "teleport gate name must not be empty"
        );
        assert!(gate.x.is_finite() && gate.z.is_finite());
        assert!(gate.rotation.is_finite());
    }
    gates
});

pub fn teleport_gates() -> &'static [TeleportGateDef] {
    &TELEPORT_GATES
}

pub fn teleport_gate_config() -> &'static TeleportGateConfig {
    &TELEPORT_GATE_CONFIG
}

pub fn teleport_gate(id: &str) -> Option<&'static TeleportGateDef> {
    teleport_gates().iter().find(|gate| gate.id == id)
}

pub fn teleport_gate_distance_m(from: &TeleportGateDef, to: &TeleportGateDef) -> f32 {
    let dx = shortest_world_delta_x(from.x, to.x);
    let dz = to.z - from.z;
    dx.hypot(dz)
}

pub fn teleport_gate_fare(from: &TeleportGateDef, to: &TeleportGateDef) -> i64 {
    let config = teleport_gate_config();
    let whole_km = (teleport_gate_distance_m(from, to) / 1_000.0).ceil() as i64;
    config.base_fare_copper + whole_km.max(1) * config.fare_per_km_copper
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_every_authored_town_gate() {
        let ids: Vec<_> = teleport_gates()
            .iter()
            .map(|gate| gate.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "aldermark",
                "brovik",
                "edra",
                "frihavn",
                "garasden",
                "mistfall",
                "riftmark",
                "stenhavn",
            ]
        );
    }

    #[test]
    fn network_config_publishes_the_shared_price_and_risk_contract() {
        let config = teleport_gate_config();
        assert_eq!(config.interaction_range_m, 6.0);
        assert_eq!(config.arrival_offset_m, 4.0);
        assert_eq!(config.base_fare_copper, 1_000);
        assert_eq!(config.fare_per_km_copper, 500);
        assert_eq!(config.misfire_chance_bps, 50);
        assert_eq!(config.dungeon_misfire_percent, 20);
    }

    #[test]
    fn fares_are_symmetric_and_increase_by_distance_band() {
        let aldermark = teleport_gate("aldermark").unwrap();
        let garasden = teleport_gate("garasden").unwrap();
        let frihavn = teleport_gate("frihavn").unwrap();

        assert_eq!(
            teleport_gate_fare(aldermark, garasden),
            teleport_gate_fare(garasden, aldermark)
        );
        assert!(teleport_gate_fare(aldermark, frihavn) > teleport_gate_fare(aldermark, garasden));
    }

    #[test]
    fn arrival_is_in_front_of_the_gate() {
        let gate = teleport_gate("aldermark").unwrap();
        let (x, z) = gate.arrival_xz();
        assert!((x - (gate.x - teleport_gate_config().arrival_offset_m)).abs() < 0.001);
        assert!((z - gate.z).abs() < 0.001);
    }
}

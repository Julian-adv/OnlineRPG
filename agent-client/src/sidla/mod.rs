//! SIDLA — a data link protocol for agent control.
//!
//! An LLM asked "what do you do?" in prose answers in prose, and prose can say
//! anything: an action that does not exist, a target that was never in sight,
//! a field the situation forbids. The driver's own parser already meets this
//! by dropping any reply it cannot read, which trades a hallucinated action
//! for a frozen NPC.
//!
//! SIDLA narrows the channel instead. World state goes out as packets rather
//! than sentences, decisions come back as packets, and every packet is checked
//! against the field matrix its header declares before anything reaches the
//! game. Three barriers stand in the way of a malformed decision:
//!
//! 1. Control fields deserialize from integers only, so a word where an enum
//!    belongs fails to parse (`packet`).
//! 2. Each header fixes which fields must, may and must not appear, so a
//!    packet carrying a field outside its purpose is rejected whole
//!    (`schema`).
//! 3. A rejected packet is answered by a deterministic ladder over the same
//!    world, so refusing a reply costs behaviour rather than removing it
//!    (`fsm`).
//!
//! What reaches the game is therefore always a packet the schema admits. The
//! guarantee is structural: it does not depend on the model's cooperation, and
//! it holds for a model that returns nothing intelligible at all.
//!
//! Determinism follows from the same arrangement. The uplink is a pure
//! function of the state snapshot, and the downlink is a pure function of the
//! packets and that snapshot, so an unchanged world and an unchanged reply
//! produce byte-identical commands. Where variety is wanted it is added
//! afterwards by `shuffle`, seeded so a run stays reproducible.
//!
//! The layer sits behind `LlmBackend` (`backend`), so it works with every
//! provider and changes nothing downstream — the driver keeps receiving the
//! same action envelope it always did.
//!
//! Headers, after the J-series message families a tactical data link uses:
//!
//! | Header | Purpose | Required | Optional |
//! | --- | --- | --- | --- |
//! | A (PPLI) | own state and position | SUB, STA, LOC | HP |
//! | B (Track) | observation and identification | SUB, TAR, IFF | REL |
//! | C (Engage) | action and interaction | SUB, TAR, ACT | MSG |
//! | D (Mission) | objective change | SUB, OBJ | TAR |
//!
//! `HP` and `MSG` are domain extensions, not part of the core dictionary: a
//! game needs to know how hurt a target is, and a talking NPC needs somewhere
//! to put the line it speaks. Both are confined to one header.

mod backend;
mod decode;
mod encode;
mod fsm;
mod packet;
mod schema;
mod shuffle;
mod wire;

pub use backend::SidlaBackend;
pub use wire::Wire;

use serde::Deserialize;

/// `[sidla]` configuration. Off unless asked for; a scenario designer opts a
/// whole fleet or a single NPC into the protocol.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SidlaConfig {
    /// Route this NPC's turns through the data link.
    pub enabled: bool,
    /// How the uplink frame is written. `compact` costs fewer tokens;
    /// `json` is easier to read in a log.
    pub wire: Wire,
    /// Vary interchangeable objectives across turns, so an agent under greedy
    /// decoding does not repeat one decision forever.
    pub shuffle: bool,
    /// Seed for that variation. A fixed seed replays a run exactly.
    pub shuffle_seed: u64,
    /// Log every uplink frame and rejected reply at debug level.
    pub log_frames: bool,
}

impl Default for SidlaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            wire: Wire::Compact,
            shuffle: false,
            shuffle_seed: 0x5344_4C41,
            log_frames: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_harness_is_off_by_default() {
        assert!(!SidlaConfig::default().enabled);
    }

    #[test]
    fn an_empty_table_yields_the_defaults() {
        let parsed: SidlaConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, SidlaConfig::default());
    }

    #[test]
    fn config_reads_every_knob() {
        let parsed: SidlaConfig = toml::from_str(
            r#"
            enabled = true
            wire = "json"
            shuffle = true
            shuffle_seed = 99
            log_frames = true
            "#,
        )
        .unwrap();
        assert_eq!(
            parsed,
            SidlaConfig {
                enabled: true,
                wire: Wire::Json,
                shuffle: true,
                shuffle_seed: 99,
                log_frames: true,
            }
        );
    }

    #[test]
    fn a_misspelled_key_is_reported_rather_than_ignored() {
        assert!(toml::from_str::<SidlaConfig>("enabeld = true").is_err());
    }

    #[test]
    fn an_unknown_wire_format_is_rejected() {
        assert!(toml::from_str::<SidlaConfig>(r#"wire = "morse""#).is_err());
    }
}

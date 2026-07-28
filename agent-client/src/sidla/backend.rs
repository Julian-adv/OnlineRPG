//! The `LlmBackend` decorator that puts the protocol on the wire.
//!
//! Wrapping the seam rather than editing the driver means the whole harness is
//! opt-in and reversible: with `[sidla] enabled = false` the wrapper is never
//! constructed and the agent behaves exactly as before.
//!
//! One turn: encode the world into an uplink frame, ask the wrapped provider,
//! validate the reply, and translate it. A reply that fails validation is
//! discarded and answered from `super::fsm` instead, so the driver downstream
//! always receives a well-formed action envelope.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::{decode, encode, fsm, schema, shuffle, wire, SidlaConfig};
use crate::driver::LlmBackend;
use crate::state::SharedState;

const PROTOCOL_BRIEF: &str = include_str!("../../data/prompts/sidla_protocol.txt");

/// Marker the driver's prompt uses for the event log. Everything above it is
/// the world state the uplink frame replaces; the events themselves stay as
/// they are, because a player's chat line is not a control field.
const EVENTS_MARKER: &str = "=== EVENTS ===";

/// Counts of what the validator did, for the operator to read.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub turns: u64,
    pub accepted: u64,
    pub discarded: u64,
}

impl Stats {
    /// Share of turns answered by the fallback rather than the provider.
    pub fn fallback_rate(&self) -> f64 {
        if self.turns == 0 {
            return 0.0;
        }
        self.discarded as f64 / self.turns as f64
    }
}

pub struct SidlaBackend {
    inner: Arc<dyn LlmBackend>,
    state: Arc<Mutex<SharedState>>,
    config: SidlaConfig,
    label: String,
    turn: AtomicU64,
    stats: std::sync::Mutex<Stats>,
}

impl SidlaBackend {
    /// Wrap `inner`, or hand it back untouched when the harness is off.
    pub fn wrap(
        inner: Arc<dyn LlmBackend>,
        state: Arc<Mutex<SharedState>>,
        config: SidlaConfig,
        label: &str,
    ) -> Arc<dyn LlmBackend> {
        if !config.enabled {
            return inner;
        }
        info!(
            "[{label}] SIDLA data link enabled (wire={:?}, shuffle={})",
            config.wire, config.shuffle
        );
        Arc::new(Self {
            inner,
            state,
            config,
            label: label.to_string(),
            turn: AtomicU64::new(0),
            stats: std::sync::Mutex::new(Stats::default()),
        })
    }

    #[cfg(test)]
    pub fn stats(&self) -> Stats {
        *self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn record(&self, accepted: bool) -> Stats {
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        stats.turns += 1;
        if accepted {
            stats.accepted += 1;
        } else {
            stats.discarded += 1;
        }
        *stats
    }

    /// Uplink frame: the protocol brief, the encoded world, and whatever event
    /// log the driver had appended to its own prompt.
    fn frame(&self, uplink: &encode::Uplink, driver_prompt: &str) -> String {
        let mut out = String::with_capacity(PROTOCOL_BRIEF.len() + 512);
        out.push_str(PROTOCOL_BRIEF);
        out.push_str("\n=== FRAME ===\n");
        out.push_str(&wire::render(&uplink.packets, self.config.wire));
        out.push('\n');
        if let Some(events) = driver_prompt.split_once(EVENTS_MARKER) {
            out.push_str("\n=== EVENTS ===");
            out.push_str(events.1.trim_end());
            out.push('\n');
        }
        out.push_str("\nReply with SIDLA packets.");
        out
    }
}

#[async_trait]
impl LlmBackend for SidlaBackend {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        let uplink = {
            let state = self.state.lock().await;
            encode::encode(&state)
        };

        let turn = self.turn.fetch_add(1, Ordering::Relaxed);
        let variation = if self.config.shuffle {
            shuffle::turn_seed(self.config.shuffle_seed, turn)
        } else {
            self.config.shuffle_seed
        };

        let frame = self.frame(&uplink, content);
        if self.config.log_frames {
            debug!("[{}] SIDLA uplink:\n{frame}", self.label);
        }

        let reply = self.inner.send_message(&frame).await?;
        let envelope = match self.transcode(&reply, &uplink, variation) {
            Ok(envelope) => {
                self.record(true);
                envelope
            }
            Err(violation) => {
                let stats = self.record(false);
                warn!(
                    "[{}] SIDLA packet discarded ({violation}); falling back \
                     ({}/{} turns, {:.0}%)",
                    self.label,
                    stats.discarded,
                    stats.turns,
                    stats.fallback_rate() * 100.0
                );
                if self.config.log_frames {
                    debug!("[{}] SIDLA rejected downlink:\n{reply}", self.label);
                }
                self.fallback(&uplink, variation)?
            }
        };
        Ok(envelope.to_string())
    }
}

impl SidlaBackend {
    fn transcode(
        &self,
        reply: &str,
        uplink: &encode::Uplink,
        variation: u64,
    ) -> Result<serde_json::Value, schema::Violation> {
        let packets = schema::parse_frame(reply)?;
        let packets: Vec<_> = if self.config.shuffle {
            packets
                .iter()
                .map(|p| shuffle::vary(p, variation))
                .collect()
        } else {
            packets
        };
        decode::to_envelope(&packets, uplink, variation)
    }

    /// The fallback must itself be translatable; if it somehow is not, the
    /// agent waits rather than acting on nothing.
    fn fallback(
        &self,
        uplink: &encode::Uplink,
        variation: u64,
    ) -> anyhow::Result<serde_json::Value> {
        let packet = fsm::decide(uplink);
        schema::validate(&packet)
            .map_err(|e| anyhow::anyhow!("SIDLA fallback produced an invalid packet: {e}"))?;
        Ok(
            decode::to_envelope(&[packet], uplink, variation).unwrap_or_else(|_| {
                serde_json::json!({
                    "thought": "SIDLA fallback: holding",
                    "actions": [{"type": "wait"}],
                })
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidla::packet::Header;
    use crate::sidla::Wire;
    use crate::state::tests::test_state;
    use onlinerpg_shared::{
        CharacterClass, ClientMessage, Monster, MonsterState, Player, PlayerId, Position,
    };
    use tokio::sync::mpsc::Receiver;

    /// Keeps the command receiver alive for the duration of a test; nothing
    /// here sends commands, but a live channel matches the real setup.
    struct World {
        state: Arc<Mutex<SharedState>>,
        _rx: Receiver<ClientMessage>,
    }

    struct Canned(String);

    #[async_trait]
    impl LlmBackend for Canned {
        async fn send_message(&self, _content: &str) -> anyhow::Result<String> {
            Ok(self.0.clone())
        }
    }

    struct Echo;

    #[async_trait]
    impl LlmBackend for Echo {
        async fn send_message(&self, content: &str) -> anyhow::Result<String> {
            Ok(content.to_string())
        }
    }

    fn pos(x: f32, z: f32) -> Position {
        Position { x, y: 0.0, z }
    }

    fn world() -> World {
        let (mut state, rx) = test_state();
        let me = Player {
            id: PlayerId::from(1),
            name: "Mika".into(),
            position: pos(0.0, 0.0),
            rotation: 0.0,
            level: 3,
            health: 100,
            max_health: 100,
            class: CharacterClass::Knight,
            gender: Default::default(),
            is_official_npc: false,
            torch_on: false,
            floor_level: 0,
            object_type: None,
            object_id: None,
            last_combat_at: 0,
            client_kind: Default::default(),
        };
        state.self_player_id = Some(me.id);
        state.self_player = Some(me);
        state.nearby_monsters.insert(
            "slime_1".into(),
            Monster {
                id: "slime_1".into(),
                monster_type: "slime".into(),
                position: pos(0.0, 5.0),
                rotation: 0.0,
                state: MonsterState::Idle,
                owner_id: None,
                health: 10,
                max_health: 10,
                floor_level: 0,
                level_override: None,
                aggressive: true,
                last_attack_at: 0,
                last_move_at: 0,
                move_budget: 0.0,
            },
        );
        World {
            state: Arc::new(Mutex::new(state)),
            _rx: rx,
        }
    }

    fn harness(reply: &str, config: SidlaConfig) -> (Arc<dyn LlmBackend>, World) {
        let world = world();
        let backend = SidlaBackend::wrap(
            Arc::new(Canned(reply.to_string())),
            Arc::clone(&world.state),
            config,
            "test",
        );
        (backend, world)
    }

    fn enabled() -> SidlaConfig {
        SidlaConfig {
            enabled: true,
            ..Default::default()
        }
    }

    async fn envelope_of(backend: &Arc<dyn LlmBackend>) -> serde_json::Value {
        let out = backend.send_message("").await.unwrap();
        serde_json::from_str(&out).expect("envelope is not JSON")
    }

    #[tokio::test]
    async fn a_disabled_harness_does_not_wrap_the_provider() {
        let world = world();
        let inner: Arc<dyn LlmBackend> = Arc::new(Canned("untouched".into()));
        let wrapped = SidlaBackend::wrap(
            Arc::clone(&inner),
            Arc::clone(&world.state),
            SidlaConfig::default(),
            "test",
        );
        assert_eq!(wrapped.send_message("anything").await.unwrap(), "untouched");
    }

    #[tokio::test]
    async fn a_valid_engage_packet_becomes_an_attack_action() {
        let (backend, _world) = harness(
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":2}"#,
            enabled(),
        );
        let envelope = envelope_of(&backend).await;
        assert_eq!(envelope["actions"][0]["type"], "attack");
        assert_eq!(envelope["actions"][0]["monster_id"], "slime_1");
    }

    /// The uplink replaces the prose world state but keeps the event log,
    /// which carries things — a player's chat line — that are not control
    /// fields and have no packet form.
    #[tokio::test]
    async fn the_frame_carries_the_brief_the_packets_and_the_events() {
        let world = world();
        let uplink = {
            let state = world.state.lock().await;
            encode::encode(&state)
        };
        let backend = SidlaBackend {
            inner: Arc::new(Echo),
            state: Arc::clone(&world.state),
            config: SidlaConfig {
                enabled: true,
                wire: Wire::Json,
                ..Default::default()
            },
            label: "test".into(),
            turn: AtomicU64::new(0),
            stats: std::sync::Mutex::new(Stats::default()),
        };
        let driver_prompt = "=== CURRENT STATE ===\n\
             You: Mika Lv.3 Knight HP 100/100 at (0.0, 0.0, 0.0)\n\
             === EVENTS ===\n[Chat] Bob: hello there";
        let frame = backend.frame(&uplink, driver_prompt);

        assert!(frame.contains("SIDLA DATA LINK PROTOCOL"));
        assert!(frame.contains(r#"{"H":"A","SUB":"Mika""#));
        assert!(frame.contains(r#"{"H":"B","SUB":"Mika","TAR":"slime_1","IFF":2"#));
        assert!(frame.contains("[Chat] Bob: hello there"));
        assert!(
            !frame.contains("Lv.3 Knight"),
            "prose world state leaked into the frame"
        );
    }

    #[tokio::test]
    async fn the_compact_wire_carries_the_same_frame_without_the_key_names() {
        let world = world();
        let uplink = {
            let state = world.state.lock().await;
            encode::encode(&state)
        };
        let backend = SidlaBackend {
            inner: Arc::new(Echo),
            state: Arc::clone(&world.state),
            config: enabled(),
            label: "test".into(),
            turn: AtomicU64::new(0),
            stats: std::sync::Mutex::new(Stats::default()),
        };
        let frame = backend.frame(&uplink, "=== CURRENT STATE ===\n");
        let packets = frame.split_once("=== FRAME ===").expect("frame section").1;
        assert!(packets.contains("A|Mika|"), "{packets}");
        assert!(packets.contains("B|Mika|slime_1|2|-100"), "{packets}");
        assert!(
            !packets.contains(r#""SUB""#),
            "json keys in a compact frame: {packets}"
        );
    }

    #[tokio::test]
    async fn a_frame_without_events_omits_the_section() {
        let world = world();
        let uplink = {
            let state = world.state.lock().await;
            encode::encode(&state)
        };
        let backend = SidlaBackend {
            inner: Arc::new(Echo),
            state: Arc::clone(&world.state),
            config: enabled(),
            label: "test".into(),
            turn: AtomicU64::new(0),
            stats: std::sync::Mutex::new(Stats::default()),
        };
        assert!(!backend
            .frame(&uplink, "=== CURRENT STATE ===\n")
            .contains("EVENTS"));
    }

    #[tokio::test]
    async fn prose_instead_of_packets_falls_back_to_a_valid_action() {
        let (backend, _world) = harness(
            "I think Mika should probably talk to the slime first.",
            enabled(),
        );
        let envelope = envelope_of(&backend).await;
        let actions = envelope["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["type"], "attack");
    }

    #[tokio::test]
    async fn a_forbidden_field_falls_back_rather_than_being_stripped() {
        let (backend, _world) = harness(
            r#"{"H":"A","SUB":"Mika","STA":0,"LOC":"Cafe","TAR":"slime_1"}"#,
            enabled(),
        );
        let envelope = envelope_of(&backend).await;
        assert!(envelope["thought"].as_str().unwrap().starts_with("SIDLA"));
        assert_eq!(envelope["actions"][0]["type"], "attack");
    }

    #[tokio::test]
    async fn an_invented_target_falls_back() {
        let (backend, _world) = harness(
            r#"{"H":"C","SUB":"Mika","TAR":"ancient_dragon","ACT":2}"#,
            enabled(),
        );
        let envelope = envelope_of(&backend).await;
        assert_eq!(envelope["actions"][0]["monster_id"], "slime_1");
    }

    #[tokio::test]
    async fn every_turn_yields_an_envelope_the_driver_can_parse() {
        let replies = [
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":2}"#,
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":"attack"}"#,
            r#"{"H":"A","SUB":"Mika","STA":4,"LOC":[0,0,0]}"#,
            r#"{"H":"D","SUB":"Mika","OBJ":1}"#,
            r#"{"H":"Z","SUB":"Mika"}"#,
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":2,"HP":50}"#,
            "",
            "```json\n{\"H\":\"C\",\"SUB\":\"Mika\",\"TAR\":\"slime_1\",\"ACT\":1,\"MSG\":\"Back!\"}\n```",
            "{}",
            "null",
            r#"{"H":"B","SUB":"Mika","TAR":"slime_1","IFF":2,"REL":-9999}"#,
        ];
        for reply in replies {
            let (backend, _world) = harness(reply, enabled());
            let out = backend.send_message("").await.unwrap();
            let envelope: serde_json::Value = serde_json::from_str(&out)
                .unwrap_or_else(|e| panic!("unparseable envelope for `{reply}`: {e}"));
            let actions = envelope["actions"]
                .as_array()
                .unwrap_or_else(|| panic!("no actions for `{reply}`"));
            assert!(!actions.is_empty(), "empty actions for `{reply}`");
            assert!(
                actions.iter().all(|a| a["type"].is_string()),
                "untyped action for `{reply}`"
            );
        }
    }

    /// The structural claim, stated as a property: over a corpus of replies
    /// that are wrong in every way a model can be wrong, no command reaching
    /// the game names an entity the world did not contain, and no turn is
    /// lost. The guarantee does not depend on the model cooperating.
    #[tokio::test]
    async fn nothing_unreal_reaches_the_game() {
        let corpus = [
            // Invented targets.
            r#"{"H":"C","SUB":"Mika","TAR":"ancient_dragon","ACT":2}"#,
            r#"{"H":"C","SUB":"Mika","TAR":"","ACT":2}"#,
            r#"{"H":"D","SUB":"Mika","OBJ":8,"TAR":"lich_king"}"#,
            // Prose where a code belongs.
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":"attack the slime"}"#,
            r#"{"H":"A","SUB":"Mika","STA":"idle","LOC":"nowhere"}"#,
            // Codes outside the dictionary.
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":42}"#,
            r#"{"H":"B","SUB":"Mika","TAR":"slime_1","IFF":-1}"#,
            // Fields the header forbids.
            r#"{"H":"A","SUB":"Mika","STA":0,"LOC":"Cafe","ACT":2,"TAR":"slime_1"}"#,
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":2,"LOC":[9,9,9]}"#,
            // Missing what the header requires.
            r#"{"H":"C","SUB":"Mika"}"#,
            r#"{"H":"D"}"#,
            // Invented fields and headers.
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":2,"SPELL":"meteor"}"#,
            r#"{"H":"X","SUB":"Mika","TAR":"slime_1","ACT":2}"#,
            // Not a packet at all.
            "Mika attacks the slime with her sword.",
            "",
            "{",
            "[]",
            "null",
            "```json\n{}\n```",
            // Well formed, but observation only.
            r#"{"H":"B","SUB":"Mika","TAR":"slime_1","IFF":2}"#,
            // Well formed and actionable — the control case.
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":2}"#,
        ];

        // The only entity in this world besides the agent itself.
        let real = ["slime_1"];

        for reply in corpus {
            let (backend, _world) = harness(reply, enabled());
            let envelope = envelope_of(&backend).await;
            let actions = envelope["actions"]
                .as_array()
                .unwrap_or_else(|| panic!("no actions array for `{reply}`"));

            assert!(!actions.is_empty(), "turn lost for `{reply}`");
            for action in actions {
                assert!(
                    action["type"].is_string(),
                    "untyped action for `{reply}`: {action}"
                );
                for key in ["monster_id", "player", "target"] {
                    if let Some(named) = action[key].as_str() {
                        assert!(
                            real.contains(&named),
                            "`{reply}` reached the game naming `{named}`"
                        );
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn the_same_reply_and_world_always_produce_the_same_envelope() {
        let (backend, _world) = harness(
            r#"{"H":"C","SUB":"Mika","TAR":"slime_1","ACT":2}"#,
            enabled(),
        );
        let first = backend.send_message("").await.unwrap();
        for _ in 0..16 {
            assert_eq!(backend.send_message("").await.unwrap(), first);
        }
    }

    #[tokio::test]
    async fn a_rejected_reply_falls_back_identically_every_time() {
        let (backend, _world) = harness("no packets here", enabled());
        let first = backend.send_message("").await.unwrap();
        for _ in 0..16 {
            assert_eq!(backend.send_message("").await.unwrap(), first);
        }
    }

    #[tokio::test]
    async fn shuffling_varies_an_idle_objective_across_turns() {
        let config = SidlaConfig {
            enabled: true,
            shuffle: true,
            shuffle_seed: 4321,
            ..Default::default()
        };
        let world = world();
        let backend = SidlaBackend::wrap(
            Arc::new(Canned(r#"{"H":"D","SUB":"Mika","OBJ":1}"#.into())),
            Arc::clone(&world.state),
            config,
            "test",
        );
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            seen.insert(envelope_of(&backend).await["actions"][0].to_string());
        }
        assert!(seen.len() > 1, "shuffling produced no variety");
    }

    #[tokio::test]
    async fn statistics_separate_accepted_turns_from_discarded_ones() {
        let world = world();
        let backend = SidlaBackend {
            inner: Arc::new(Canned("prose".into())),
            state: Arc::clone(&world.state),
            config: enabled(),
            label: "test".into(),
            turn: AtomicU64::new(0),
            stats: std::sync::Mutex::new(Stats::default()),
        };
        for _ in 0..4 {
            backend.send_message("").await.unwrap();
        }
        let stats = backend.stats();
        assert_eq!(stats.turns, 4);
        assert_eq!(stats.discarded, 4);
        assert_eq!(stats.accepted, 0);
        assert!((stats.fallback_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn a_provider_error_is_not_swallowed_by_the_fallback() {
        struct Failing;
        #[async_trait]
        impl LlmBackend for Failing {
            async fn send_message(&self, _content: &str) -> anyhow::Result<String> {
                Err(anyhow::anyhow!("endpoint down"))
            }
        }
        let world = world();
        let backend = SidlaBackend::wrap(
            Arc::new(Failing),
            Arc::clone(&world.state),
            enabled(),
            "test",
        );
        assert!(backend.send_message("").await.is_err());
    }

    #[tokio::test]
    async fn the_uplink_frame_is_schema_valid_and_header_split() {
        let world = world();
        let uplink = {
            let s = world.state.lock().await;
            encode::encode(&s)
        };
        assert!(uplink.packets.iter().any(|p| p.h == Header::A));
        assert!(uplink.packets.iter().any(|p| p.h == Header::B));
        for packet in &uplink.packets {
            schema::validate(packet).unwrap();
        }
    }
}

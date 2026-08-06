//! Ambient track list from `data/bgm.json`, mirroring the server's
//! `bgm_defs.rs`. The agent has no audio, so a performance it starts is timed
//! from `seconds` — that is what tells it when to stop strumming.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct BgmTrack {
    pub seconds: u64,
}

fn tracks() -> &'static HashMap<String, BgmTrack> {
    static CACHE: OnceLock<HashMap<String, BgmTrack>> = OnceLock::new();
    CACHE.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/bgm.json")).unwrap_or_default()
    })
}

/// How long `track` runs. An unknown title means the registries drifted apart;
/// a couple of minutes is a better guess than strumming forever.
pub fn duration(track: &str) -> Duration {
    const UNKNOWN_TRACK_SECS: u64 = 120;
    Duration::from_secs(
        tracks()
            .get(track)
            .map_or(UNKNOWN_TRACK_SECS, |t| t.seconds),
    )
}

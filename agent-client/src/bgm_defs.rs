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

/// Whether a `/play_music` argument names a track. Which one it lands on is
/// the server's business; this only answers whether it would say "No such
/// song", by the server's own rule.
pub fn knows(query: &str) -> bool {
    onlinerpg_shared::messages::resolve_title(tracks().keys().map(String::as_str), query).is_some()
}

/// Titles a bard can announce before playing. The registry's variants —
/// "… (1)", "… 2", "… (Epic Orchestral Version)" — are folded into their base
/// title: nobody announces a track number, and the server resolves a base by
/// fragment even when only the variant exists.
pub fn songbook() -> Vec<String> {
    let mut titles: Vec<String> = tracks().keys().map(|t| base_title(t).to_string()).collect();
    titles.sort();
    titles.dedup();
    titles
}

fn base_title(title: &str) -> &str {
    let title = match title.strip_suffix(')').and_then(|t| t.rfind('(')) {
        Some(open) => title[..open].trim_end(),
        None => title,
    };
    match title.rsplit_once(' ') {
        Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => title,
    }
}

#[cfg(test)]
mod tests {
    use super::{base_title, songbook, tracks};

    #[test]
    fn the_songbook_folds_variants_into_titles_a_bard_can_say() {
        let book = songbook();
        assert!(book.contains(&"Triumphal Procession".to_string()));
        assert!(!book.iter().any(|t| t.contains('(') || t.ends_with(" 2")));
        // Only the "(1)" variant is in the registry; the base still resolves.
        assert!(book.contains(&"Wanderer of the Old Fields".to_string()));
        assert!(
            book.windows(2).all(|w| w[0] < w[1]),
            "sorted, no duplicates"
        );
        // Every entry resolves server-side: whole title or fragment of one.
        assert!(book
            .iter()
            .all(|t| tracks().keys().any(|full| full.contains(t))));
    }

    #[test]
    fn a_title_that_only_looks_like_a_variant_is_left_alone() {
        assert_eq!(
            base_title("Shadowed Keep in G Minor"),
            "Shadowed Keep in G Minor"
        );
        assert_eq!(
            base_title("Castle Glass & Copper Skies"),
            "Castle Glass & Copper Skies"
        );
    }
}

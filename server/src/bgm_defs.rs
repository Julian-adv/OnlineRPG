use rand::Rng;
use std::collections::HashMap;
use std::sync::OnceLock;
use tracing::info;

/// Ambient track titles from the game-data registry (`data/bgm.json`). The
/// server only ever hands out titles — the web client maps them to files, and
/// the agent-client maps them to durations, each from its own copy of the
/// registry.
pub struct BgmDefs {
    titles: Vec<String>,
}

impl BgmDefs {
    fn load() -> Self {
        let data = include_str!("../../data/bgm.json");
        let by_id: HashMap<String, serde::de::IgnoredAny> =
            serde_json::from_str(data).expect("Failed to parse bgm.json");
        let mut titles: Vec<String> = by_id.into_keys().collect();
        // Registry order is a HashMap's, so sort for a stable random pick.
        titles.sort();
        info!("Loaded {} BGM track(s)", titles.len());
        Self { titles }
    }

    /// The track a `/play_music` argument asks for. An empty query picks at
    /// random; a title matches whole or as a fragment, ignoring case. Every
    /// client resolves through the server, so an agent-client that knows no
    /// titles can still strike up a tune.
    pub fn resolve(&self, query: &str) -> Option<&str> {
        if query.trim().is_empty() {
            let i = rand::thread_rng().gen_range(0..self.titles.len());
            return Some(self.titles[i].as_str());
        }
        onlinerpg_shared::messages::resolve_title(self.titles.iter().map(String::as_str), query)
    }
}

pub fn bgm_defs() -> &'static BgmDefs {
    static DEFS: OnceLock<BgmDefs> = OnceLock::new();
    DEFS.get_or_init(BgmDefs::load)
}

#[cfg(test)]
mod tests {
    use super::bgm_defs;

    #[test]
    fn a_title_matches_whole_or_in_part_and_a_blank_query_picks_something() {
        assert_eq!(
            bgm_defs().resolve("twilight fields"),
            Some("Twilight Fields")
        );
        assert_eq!(
            bgm_defs().resolve("lantern"),
            Some("Festival in the Lantern Square")
        );
        // The whole title wins over the longer ones containing it.
        assert_eq!(
            bgm_defs().resolve("Triumphal Procession"),
            Some("Triumphal Procession")
        );
        assert!(bgm_defs().resolve("").is_some());
        assert_eq!(bgm_defs().resolve("nonesuch"), None);
        // Battle music is not in the registry, so nobody can call it up.
        assert_eq!(bgm_defs().resolve("Blood and Bronze"), None);
    }
}

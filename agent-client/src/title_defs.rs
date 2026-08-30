//! Title names from data/titles.json (doc/TITLES.md), for the prompt.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
struct TitleDef {
    name: String,
}

static DEFS: LazyLock<HashMap<String, TitleDef>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../data/titles.json")).unwrap_or_default()
});

/// Display name, falling back to the id for one this build doesn't know.
pub fn title_name(id: &str) -> &str {
    DEFS.get(id).map_or(id, |d| d.name.as_str())
}

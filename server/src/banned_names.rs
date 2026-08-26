use std::collections::HashSet;
use std::path::Path;
use tracing::{info, warn};

/// Trimmed, case-folded comparison form.
pub fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Names no player character may take, one per line with `#` comments. Read
/// at startup rather than compiled in, so the list grows with a restart
/// instead of a deploy. Unreadable means empty: this is moderation, not a
/// security gate. See doc/CHARACTER_NAMES.md.
pub fn load(path: &Path) -> HashSet<String> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        warn!("No banned names from {}: {}", path.display(), e);
        String::new()
    });

    let names: HashSet<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(normalize)
        .collect();
    info!(
        "Loaded {} banned character name(s) from {}",
        names.len(),
        path.display()
    );
    names
}

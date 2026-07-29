//! Dungeon entrance registry, embedded at compile time from
//! `data-src/dungeons.csv`. The entrance id seeds the deterministic layout
//! generator, and the footprint test below is what the server's
//! `validated_dungeon_floor` accepts or rejects a claimed dungeon floor by —
//! so it is a wire contract, not just a lookup. Every Rust side (server,
//! agent-client) reads it from here; the browser embeds the generated
//! `data/dungeons.json` at vite build instead.
//!
//! We read the SOURCE csv rather than that generated JSON for the same reason
//! `SPAWN_TABLES` does: the JSON is produced by a build script whose ordering
//! relative to this crate isn't guaranteed (and this crate has none of its
//! own). Rows keep csv order, which is stable and identical on both sides.

use std::sync::LazyLock;

use super::{dungeon_origin, GRID};
use crate::world::Position;

/// One dungeon entrance: where it stands, how deep it goes, what its final
/// chest holds.
#[derive(Debug, Clone)]
pub struct DungeonEntranceDef {
    pub id: String,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Entrance orientation. Consumed by the client to face the entrance
    /// building; nothing Rust-side needs it.
    pub rotation: f32,
    /// Item definition ids the final-floor treasure chest always yields.
    /// Validated against the item table by the server on load.
    pub chest_drops: Vec<String>,
    /// Fixed floor count from the csv; `None` = seed-derived 5..=20.
    pub floors: Option<u8>,
    /// Monster type guarding the final-floor chest; blank falls back to
    /// [`super::BOSS_MONSTER_TYPE`]. Validated against the monster table by
    /// the server on load. Part of the hashed layout — changing it desyncs
    /// deployed clients like any generation change.
    pub boss: String,
    /// Random chest-loot ceiling: the pool only admits items whose
    /// `chestTier` (items.csv) is at or below this. Blank = 1. Server-side
    /// only — never touches layout generation.
    pub chest_tier: u8,
}

impl DungeonEntranceDef {
    pub fn position(&self) -> Position {
        Position {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }

    /// Whether (x, z) lies inside this dungeon's grid footprint.
    pub fn footprint_contains(&self, x: f32, z: f32) -> bool {
        footprint_contains(self.x, self.z, x, z)
    }
}

/// Whether (x, z) lies inside the [`GRID`] footprint of a dungeon entered at
/// (`entrance_x`, `entrance_z`). Single definition on purpose: the server
/// decides whether a client is underground by this test, so a client that
/// answers it differently gets its floor silently reset mid-descent.
pub fn footprint_contains(entrance_x: f32, entrance_z: f32, x: f32, z: f32) -> bool {
    let (ox, oz) = dungeon_origin(entrance_x, entrance_z);
    x >= ox && x < ox + GRID as f32 && z >= oz && z < oz + GRID as f32
}

static ENTRANCES: LazyLock<Vec<DungeonEntranceDef>> =
    LazyLock::new(|| parse_entrances(include_str!("../../../data-src/dungeons.csv")));

fn parse_entrances(csv: &str) -> Vec<DungeonEntranceDef> {
    let mut lines = csv.lines();
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let cols: Vec<&str> = header.split(',').map(str::trim).collect();
    let col = |name: &str| {
        cols.iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("dungeons.csv missing `{name}` column"))
    };
    lines
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let fields: Vec<&str> = line.split(',').collect();
            let field = |name: &str| fields.get(col(name)).map(|s| s.trim()).unwrap_or("");
            let id = field("id");
            if id.is_empty() {
                return None;
            }
            let coord = |name: &str| {
                field(name)
                    .parse::<f32>()
                    .unwrap_or_else(|_| panic!("dungeon '{id}' has a non-numeric {name}"))
            };
            Some(DungeonEntranceDef {
                id: id.to_string(),
                name: field("name").to_string(),
                x: coord("x"),
                y: coord("y"),
                z: coord("z"),
                rotation: field("rotation").parse().unwrap_or(0.0),
                chest_drops: field("chestDrops")
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
                // Blank (or unparseable) means "seed-derived depth".
                floors: field("floors").parse::<u8>().ok(),
                boss: match field("boss") {
                    "" => super::BOSS_MONSTER_TYPE.to_string(),
                    b => b.to_string(),
                },
                chest_tier: field("chestTier").parse().unwrap_or(1),
            })
        })
        .collect()
}

/// Every registry entrance, in csv row order.
pub fn entrances() -> &'static [DungeonEntranceDef] {
    &ENTRANCES
}

pub fn entrance(id: &str) -> Option<&'static DungeonEntranceDef> {
    ENTRANCES.iter().find(|d| d.id == id)
}

/// Entrance whose grid footprint contains the given XZ position.
pub fn entrance_at(x: f32, z: f32) -> Option<&'static DungeonEntranceDef> {
    ENTRANCES.iter().find(|d| d.footprint_contains(x, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_drops_lists_and_optional_floor_override() {
        let csv = "id,name,x,y,z,rotation,chestDrops,floors,boss,chestTier\n\
                   a,A Place,-1450,0.7,4720,90,shield;armor,5,orc_boss,2\n\
                   b,B Place,10,0,20,,,,,\n";
        let defs = parse_entrances(csv);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].chest_drops, ["shield", "armor"]);
        assert_eq!(defs[0].floors, Some(5));
        assert_eq!(defs[0].rotation, 90.0);
        assert_eq!(defs[0].boss, "orc_boss");
        assert_eq!(defs[0].chest_tier, 2);
        assert!(defs[1].chest_drops.is_empty());
        assert_eq!(defs[1].floors, None, "blank floors = seed-derived depth");
        assert_eq!(defs[1].boss, super::super::BOSS_MONSTER_TYPE);
        assert_eq!(defs[1].chest_tier, 1, "blank chestTier = tier 1");
    }

    /// Half-open on both axes, and the embedded csv parses at all. The server
    /// admits a client underground by exactly this test, so its edges are part
    /// of the wire contract.
    #[test]
    fn footprint_is_half_open_around_the_entrance() {
        let def = entrances().first().expect("a registry entrance");
        assert!(def.footprint_contains(def.x, def.z));
        assert!(entrance_at(def.x, def.z).is_some());
        let (ox, oz) = dungeon_origin(def.x, def.z);
        assert!(def.footprint_contains(ox, oz));
        assert!(!def.footprint_contains(ox - 0.01, oz));
        assert!(!def.footprint_contains(ox, oz + GRID as f32));
    }
}

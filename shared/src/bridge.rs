//! Bridge decks as flat XZ rectangles, from the object catalog's `bridge`
//! metadata. The server has no deck heights; it only needs "is this point on
//! a deck" so a river crossing by bridge is not a wade.

use crate::furniture::FurniturePlacement;
use std::collections::HashMap;
use std::sync::OnceLock;

static CATALOG_JSON: &str = include_str!("../../client/public/models/objects/catalog.json");

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeckRect {
    deck_min_x: f32,
    deck_max_x: f32,
    deck_min_z: f32,
    deck_max_z: f32,
}

#[derive(serde::Deserialize)]
struct CatalogEntry {
    id: String,
    bridge: Option<DeckRect>,
}

fn decks() -> &'static HashMap<String, DeckRect> {
    static TABLE: OnceLock<HashMap<String, DeckRect>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let entries: Vec<CatalogEntry> =
            serde_json::from_str(CATALOG_JSON).expect("objects/catalog.json is malformed");
        entries
            .into_iter()
            .filter_map(|e| Some((e.id, e.bridge?)))
            .collect()
    })
}

fn deck_rect(type_id: &str) -> Option<DeckRect> {
    decks().get(type_id).copied()
}

/// A placed deck: the local rect plus the placement's yaw. Mirrors the
/// browser's `bridgeManager.findBridgeAt`.
#[derive(Clone, Debug)]
pub struct PlacedDeck {
    px: f32,
    pz: f32,
    cos: f32,
    sin: f32,
    rect: DeckRect,
}

impl PlacedDeck {
    fn new(p: &FurniturePlacement, rect: DeckRect) -> Self {
        let (sin, cos) = p.rotation_deg.to_radians().sin_cos();
        Self {
            px: p.x,
            pz: p.z,
            cos,
            sin,
            rect,
        }
    }

    pub fn contains(&self, wx: f32, wz: f32) -> bool {
        let dx = wx - self.px;
        let dz = wz - self.pz;
        let lx = dx * self.cos - dz * self.sin;
        let lz = dx * self.sin + dz * self.cos;
        lx >= self.rect.deck_min_x
            && lx <= self.rect.deck_max_x
            && lz >= self.rect.deck_min_z
            && lz <= self.rect.deck_max_z
    }
}

/// Every bridge among a region's placements.
pub fn placed_decks(placements: &[FurniturePlacement]) -> Vec<PlacedDeck> {
    placements
        .iter()
        .filter_map(|p| deck_rect(&p.type_id).map(|r| PlacedDeck::new(p, r)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(type_id: &str, x: f32, z: f32, rotation_deg: f32) -> FurniturePlacement {
        FurniturePlacement {
            type_id: type_id.into(),
            x,
            y: 0.0,
            z,
            rotation_deg,
            floor_level: 0,
        }
    }

    #[test]
    fn catalog_lists_the_bridges() {
        let r = deck_rect("stone_bridge").unwrap();
        assert!(r.deck_max_z > 10.0 && deck_rect("bed").is_none());
    }

    #[test]
    fn rotated_deck_contains_points_along_its_axis() {
        let decks = placed_decks(&[place("stone_bridge", 100.0, 50.0, 90.0)]);
        assert_eq!(decks.len(), 1);
        let d = &decks[0];
        assert!(d.contains(109.0, 50.0) && d.contains(91.0, 50.5));
        assert!(!d.contains(100.0, 59.0) && !d.contains(111.0, 50.0));
    }
}

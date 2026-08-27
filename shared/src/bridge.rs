//! Bridge decks from the object catalog's `bridge` metadata: the deck's XZ
//! rectangle and its sampled height curve, so the server can place a mover
//! on a deck without trusting the client's Y.

use crate::furniture::FurniturePlacement;
use std::collections::HashMap;
use std::sync::OnceLock;

static CATALOG_JSON: &str = include_str!("../../client/public/models/objects/catalog.json");

/// A mover this close to a deck's height counts as on it
/// (`bridgeManager.DECK_Y_TOLERANCE`).
const DECK_Y_TOLERANCE_M: f32 = 1.5;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeckRect {
    deck_min_x: f32,
    deck_max_x: f32,
    deck_min_z: f32,
    deck_max_z: f32,
    deck_crown_y: f32,
    deck_axis: DeckAxis,
    #[serde(default)]
    deck_y_samples: Vec<f32>,
    /// Derived once in `decks()`: half the deck's length along its axis,
    /// and its lowest height (the abutments).
    #[serde(skip)]
    half_len: f32,
    #[serde(skip)]
    base_local_y: f32,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum DeckAxis {
    X,
    Z,
}

impl DeckRect {
    fn derive(mut self) -> Self {
        self.half_len = match self.deck_axis {
            DeckAxis::Z => self.deck_min_z.abs().max(self.deck_max_z.abs()),
            DeckAxis::X => self.deck_min_x.abs().max(self.deck_max_x.abs()),
        };
        self.base_local_y = self
            .deck_y_samples
            .iter()
            .copied()
            .fold(self.deck_crown_y, f32::min);
        self
    }

    /// Deck-top height above the placement at local `(lx, lz)`, from the
    /// sampled curve along the deck axis (the browser's fallback before its
    /// GLB raycast; the two agree to within `DECK_Y_TOLERANCE_M`).
    fn local_y(&self, lx: f32, lz: f32) -> f32 {
        let along = match self.deck_axis {
            DeckAxis::Z => lz,
            DeckAxis::X => lx,
        };
        let half = self.half_len;
        let samples = &self.deck_y_samples;
        if half <= 0.0 || samples.len() < 2 {
            return self.deck_crown_y;
        }
        let last = samples.len() - 1;
        let f = (along.abs() / half).min(1.0) * last as f32;
        let i = (f.floor() as usize).min(last - 1);
        samples[i] + (samples[i + 1] - samples[i]) * (f - i as f32)
    }
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
            .filter_map(|e| Some((e.id, e.bridge?.derive())))
            .collect()
    })
}

fn deck_rect(type_id: &str) -> Option<&'static DeckRect> {
    decks().get(type_id)
}

/// A placed deck: the local rect plus the placement's yaw. Mirrors the
/// browser's `bridgeManager.findBridgeAt`.
#[derive(Clone, Copy, Debug)]
pub struct PlacedDeck {
    px: f32,
    py: f32,
    pz: f32,
    cos: f32,
    sin: f32,
    rect: &'static DeckRect,
}

impl PlacedDeck {
    fn new(p: &FurniturePlacement, rect: &'static DeckRect) -> Self {
        let (sin, cos) = p.rotation_deg.to_radians().sin_cos();
        Self {
            px: p.x,
            py: p.y,
            pz: p.z,
            cos,
            sin,
            rect,
        }
    }

    /// World deck-top Y at `(wx, wz)`, if the point is on this deck.
    pub fn deck_y(&self, wx: f32, wz: f32) -> Option<f32> {
        let dx = wx - self.px;
        let dz = wz - self.pz;
        let lx = dx * self.cos - dz * self.sin;
        let lz = dx * self.sin + dz * self.cos;
        let r = self.rect;
        (lx >= r.deck_min_x && lx <= r.deck_max_x && lz >= r.deck_min_z && lz <= r.deck_max_z)
            .then(|| self.py + r.local_y(lx, lz))
    }

    /// Deck-top Y for a mover at server height `ref_y` standing on this deck,
    /// rather than in the river or ravine beneath it: anyone no lower than the
    /// abutments (within tolerance) is on the bridge — a wader under the span
    /// is metres below them, a walker on the bank level with them.
    pub fn stand_y(&self, wx: f32, wz: f32, ref_y: f32) -> Option<f32> {
        if ref_y < self.py + self.rect.base_local_y - DECK_Y_TOLERANCE_M {
            return None;
        }
        self.deck_y(wx, wz)
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
        assert!(r.deck_max_z > 10.0 && r.deck_y_samples.len() == 11);
        assert!(deck_rect("bed").is_none());
    }

    #[test]
    fn rotated_deck_covers_points_along_its_axis() {
        let decks = placed_decks(&[place("stone_bridge", 100.0, 50.0, 90.0)]);
        assert_eq!(decks.len(), 1);
        let d = &decks[0];
        assert!(d.deck_y(109.0, 50.0).is_some() && d.deck_y(91.0, 50.5).is_some());
        assert!(d.deck_y(100.0, 59.0).is_none() && d.deck_y(111.0, 50.0).is_none());
    }

    #[test]
    fn deck_height_follows_the_arch() {
        let mut p = place("stone_bridge", 0.0, 0.0, 0.0);
        p.y = 3.0;
        let d = &placed_decks(&[p])[0];
        let crown = d.deck_y(0.0, 0.0).unwrap();
        let end = d.deck_y(0.0, 10.13).unwrap();
        assert!((crown - 5.4951).abs() < 1e-3, "{crown}");
        assert!((end - 3.0006).abs() < 1e-3, "{end}");
        assert!(d.deck_y(0.0, 5.0).unwrap() < crown);
    }

    #[test]
    fn only_movers_level_with_the_abutments_stand_on_the_deck() {
        let mut p = place("stone_bridge", 0.0, 0.0, 0.0);
        p.y = 3.0;
        let d = &placed_decks(&[p])[0];
        assert!(d.stand_y(0.0, 0.0, 3.0).is_some());
        assert!(d.stand_y(0.0, 0.0, 1.6).is_some());
        assert!(d.stand_y(0.0, 0.0, -2.0).is_none());
    }
}

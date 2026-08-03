use onlinerpg_shared::inventory::EquipSlot;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// Chest roll chance for items whose home tier is below the dungeon's
/// (doc/ITEM_TIERS.md "하위 티어 이월템").
const CHEST_CARRYOVER_CHANCE: f32 = 0.10;

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ItemDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub weight: f32,
    #[serde(rename = "equipSlot")]
    pub equip_slot: Option<EquipSlot>,
    #[serde(default)]
    pub stackable: bool,
    #[serde(rename = "worldModel")]
    pub world_model: Option<String>,
    /// Item kind that decides how `dice` is interpreted ("weapon" → damage,
    /// "consumable" → healing) plus broad classification (armor, accessory,
    /// currency).
    #[serde(default)]
    pub category: Option<String>,
    /// Dice notation (e.g. "1d8", "6d4") whose meaning depends on `category`.
    /// Read it through `damage_dice()` / `heal_dice()` rather than directly.
    #[serde(default)]
    pub dice: Option<String>,
    #[serde(default)]
    pub material: Option<String>,
    /// Base price in the smallest currency unit. Items without a price
    /// cannot be bought or sold.
    #[serde(rename = "basePrice")]
    pub base_price: Option<i64>,
    /// Guard (AC) bonus granted while this item is equipped. Summed across all
    /// equipped items and added to the wearer's base guard when attacked.
    #[serde(default)]
    pub guard: Option<i32>,
    /// Fish only — rarity tier 1 (common) … 5 (legendary). Drives catch
    /// weighting and skill XP (doc/FISHING.md).
    #[serde(rename = "rarityTier", default)]
    pub rarity_tier: Option<u32>,
    /// Fish only — relative weight in the catch table at fishing level 0.
    #[serde(rename = "catchWeight", default)]
    pub catch_weight: Option<u32>,
    /// Fish only — the fishing level a catch is locked behind. Absent or 0
    /// means available from the first cast.
    #[serde(rename = "minFishingLevel", default)]
    pub min_fishing_level: Option<u32>,
    /// Fish only — dice notation for rolled length in centimeters.
    #[serde(rename = "sizeDice", default)]
    pub size_dice: Option<String>,
    /// Fish only — rolled length at or above this is a trophy catch.
    #[serde(rename = "trophyCm", default)]
    pub trophy_cm: Option<u32>,
    /// Equipment only — the dungeon `chestTier` (dungeons.csv) this drops at.
    /// Opt-in: absent means never in any chest pool (doc/ITEM_TIERS.md).
    #[serde(rename = "chestTier", default)]
    pub chest_tier: Option<u8>,
    /// Per-chest-open roll chance at the item's home tier. Absent = 0
    /// (signature drops are guaranteed via dungeons.csv `chestDrops` instead).
    #[serde(rename = "chestChance", default)]
    pub chest_chance: Option<f32>,
    /// Usable from the bag. The clients read this flag; `load()` fails the
    /// boot if it ever disagrees with the `use_effect` dispatch.
    #[serde(default)]
    pub consumable: bool,
    /// Satiation restored when eaten (doc/HUNGER.md). Present on food and fish.
    #[serde(default)]
    pub nutrition: Option<u32>,
    /// Fish only — the item def this grills into at a campfire.
    #[serde(rename = "grillsInto", default)]
    pub grills_into: Option<String>,
}

/// The effect produced by consuming a usable item via `use_item`, decided by
/// the item's `category`. One place to extend when a new consumable lands.
pub enum UseEffect {
    /// Restore HP by rolling the given dice notation.
    Heal(String),
    /// Restore satiation (doc/HUNGER.md). Fish also heal by their dice, and
    /// raw fish risk food poisoning — unless grilled first at a campfire.
    Eat {
        nutrition: u32,
        heal_dice: Option<String>,
        raw_fish: bool,
    },
    /// Light a campfire where the user stands.
    PlaceCampfire,
    /// Teleport the user back to the town spawn point.
    TeleportTown,
    /// Add +1 enchantment to the wielded weapon (NetHack style).
    EnchantWeapon,
    /// Ask every party member to teleport to the reader's side.
    SummonParty,
    /// Open a fished-up coin pouch: roll the given dice for its copper.
    OpenCoinPouch(String),
}

impl ItemDefinition {
    pub fn is_weapon(&self) -> bool {
        self.category.as_deref() == Some("weapon")
    }

    /// Main-hand tool that enables casting (`ClientMessage::FishingCast`).
    /// Not a weapon: no damage dice, so attacking with it rod-in-hand uses
    /// the bare-handed path.
    pub fn is_fishing_rod(&self) -> bool {
        self.category.as_deref() == Some("fishing_rod")
    }

    pub fn is_fish(&self) -> bool {
        self.category.as_deref() == Some("fish")
    }

    /// A catch that lands in the bag sealed and pays out coins when opened
    /// (`use_item`). Its `dice` column is the copper roll (the
    /// category-decides-meaning pattern: weapon → damage, fish/potion →
    /// heal, coin_catch → gold). Production code dispatches through
    /// `use_effect`; the tests keep this named predicate for the economy
    /// guardrail.
    #[cfg(test)]
    pub fn is_coin_catch(&self) -> bool {
        self.category.as_deref() == Some("coin_catch")
    }

    /// Whether a catch of this item at `size_cm` is a trophy. Trophies are
    /// a fish concept — a nat-20 Old Boot is still just a (very large) boot —
    /// and fire on the natural-20 quality roll or on meeting `trophyCm`.
    pub fn trophy_at(&self, size_cm: u16, nat_twenty: bool) -> bool {
        self.is_fish()
            && (nat_twenty
                || self
                    .trophy_cm
                    .is_some_and(|threshold| u32::from(size_cm) >= threshold))
    }

    /// Damage dice if this item is a weapon, else `None`.
    pub fn damage_dice(&self) -> Option<&str> {
        if self.is_weapon() {
            self.dice.as_deref()
        } else {
            None
        }
    }

    /// The effect of using this item from the bag, or `None` if it isn't a
    /// consumable.
    pub fn use_effect(&self) -> Option<UseEffect> {
        match self.category.as_deref()? {
            "healing_potion" => self.dice.clone().map(UseEffect::Heal),
            // Eating a fish heals by its dice and feeds a little — raw.
            "fish" => Some(UseEffect::Eat {
                nutrition: self
                    .nutrition
                    .unwrap_or(onlinerpg_shared::hunger::RAW_FISH_NUTRITION),
                heal_dice: self.dice.clone(),
                raw_fish: true,
            }),
            "food" => self.nutrition.map(|nutrition| UseEffect::Eat {
                nutrition,
                heal_dice: None,
                raw_fish: false,
            }),
            "campfire_kit" => Some(UseEffect::PlaceCampfire),
            "return_scroll" => Some(UseEffect::TeleportTown),
            "enchant_scroll" => Some(UseEffect::EnchantWeapon),
            "party_summon_scroll" => Some(UseEffect::SummonParty),
            "coin_catch" => self.dice.clone().map(UseEffect::OpenCoinPouch),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ItemDefs {
    defs: Arc<HashMap<String, ItemDefinition>>,
    /// Precomputed at load — defs are immutable, and bites roll every ~5-12 s
    /// per angler.
    catch_table: Arc<Vec<crate::game_state::fishing::CatchCandidate>>,
}

impl ItemDefs {
    pub fn load() -> Self {
        let data = include_str!("../../data/items.json");
        let defs: HashMap<String, ItemDefinition> =
            serde_json::from_str(data).expect("Failed to parse items.json");

        // A chestTier opt-in on a non-equippable or a fishing rod (bought
        // tool, doc/FISHING.md) is a data error — fail the boot, don't
        // silently filter it out of every chest.
        for def in defs.values() {
            assert!(
                def.chest_tier.is_none() || (def.equip_slot.is_some() && !def.is_fishing_rod()),
                "item '{}' has a chestTier but is not chest-eligible equipment",
                def.id
            );
            // The clients' bag-use UX keys off the CSV flag; the server acts
            // on `use_effect`. Fail the boot the moment they disagree.
            assert!(
                def.consumable == def.use_effect().is_some(),
                "item '{}': consumable flag out of step with its use_effect",
                def.id
            );
        }

        info!("Loaded {} item definitions", defs.len());
        for (id, def) in &defs {
            info!(
                "  {} - weight:{} equipSlot:{:?} stackable:{}",
                id, def.weight, def.equip_slot, def.stackable
            );
        }

        let mut catch_table: Vec<_> = defs
            .values()
            .filter_map(|def| {
                Some(crate::game_state::fishing::CatchCandidate {
                    item_def_id: def.id.clone(),
                    rarity: def.rarity_tier.unwrap_or(1),
                    catch_weight: def.catch_weight?,
                    min_fishing_level: def.min_fishing_level.unwrap_or(0),
                })
            })
            .collect();
        catch_table.sort_by(|a, b| a.item_def_id.cmp(&b.item_def_id));

        Self {
            defs: Arc::new(defs),
            catch_table: Arc::new(catch_table),
        }
    }

    pub fn get(&self, item_def_id: &str) -> Option<&ItemDefinition> {
        self.defs.get(item_def_id)
    }

    pub fn all(&self) -> impl Iterator<Item = &ItemDefinition> {
        self.defs.values()
    }

    pub fn item_def_id_for_weapon_ref(&self, weapon_ref: &str) -> Option<String> {
        if self.defs.contains_key(weapon_ref) {
            return Some(weapon_ref.to_string());
        }

        if let Some(item_id) = weapon_ref
            .strip_suffix(".glb")
            .filter(|item_id| self.defs.contains_key(*item_id))
        {
            return Some(item_id.to_string());
        }

        self.defs
            .values()
            .find(|def| def.world_model.as_deref() == Some(weapon_ref))
            .map(|def| def.id.clone())
    }

    pub fn damage_dice_for_weapon_model(&self, weapon_model: &str) -> Option<String> {
        self.item_def_id_for_weapon_ref(weapon_model)
            .and_then(|item_id| self.defs.get(&item_id))
            .and_then(|def| def.damage_dice().map(str::to_string))
    }

    /// The chest roll table for a dungeon tier: every opted-in item
    /// (`chestTier` set) at or below the tier, paired with its per-open roll
    /// chance — its own `chestChance` at its home tier, a flat carryover
    /// chance below it so missed pieces can still be filled in upstairs.
    /// Independent per-item rolls keep set-completion odds stable as the
    /// pool grows (doc/ITEM_TIERS.md). Sorted for determinism. `chestTier`
    /// is the sole membership predicate — `load` rejects opt-ins that are
    /// not chest-eligible equipment.
    pub fn chest_roll_table(&self, dungeon_tier: u8) -> Vec<(String, f32)> {
        let mut rows: Vec<(String, f32)> = self
            .defs
            .values()
            .filter_map(|def| {
                let tier = def.chest_tier?;
                let chance = if tier == dungeon_tier {
                    def.chest_chance.unwrap_or(0.0)
                } else if tier < dungeon_tier {
                    CHEST_CARRYOVER_CHANCE
                } else {
                    return None;
                };
                Some((def.id.clone(), chance))
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    pub fn weight(&self, item_def_id: &str) -> f32 {
        self.defs.get(item_def_id).map(|d| d.weight).unwrap_or(1.0)
    }

    /// The fishing catch table: every item def with a `catchWeight` — fish,
    /// junk flotsam (rarityTier 0 → no skill XP), and coin catches alike.
    /// Sorted by id for a deterministic cumulative walk.
    pub fn catch_table(&self) -> &[crate::game_state::fishing::CatchCandidate] {
        &self.catch_table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onlinerpg_shared::skills::SKILL_LEVEL_CAP;

    fn table_ids(defs: &ItemDefs, tier: u8) -> Vec<String> {
        defs.chest_roll_table(tier)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn fishing_rod_is_not_dungeon_chest_treasure() {
        // Rods are bought, not looted from bosses: none carries a chestTier,
        // and load() fails the boot if one is ever given it.
        let defs = ItemDefs::load();
        let pool = table_ids(&defs, u8::MAX);
        assert!(
            !pool.contains(&"fishing_rod".to_string()),
            "fishing rod must not be in the dungeon chest loot pool"
        );
        // Sanity: opted-in combat gear still is.
        assert!(
            pool.contains(&"iron_boots".to_string()),
            "expected iron_boots in the chest pool"
        );
    }

    /// Pool membership and chances are laws derived from the defs, so adding
    /// an item can't stale this test. The doc/ITEM_TIERS.md placement is
    /// pinned only by the debut anchors, which move when the design does.
    #[test]
    fn chest_tiers_gate_endgame_loot_by_dungeon() {
        let defs = ItemDefs::load();
        let max_tier = defs.all().filter_map(|d| d.chest_tier).max().unwrap();

        for tier in 1..=max_tier {
            let mut expected: Vec<String> = defs
                .all()
                .filter(|def| def.chest_tier.is_some_and(|home| home <= tier))
                .map(|def| def.id.clone())
                .collect();
            expected.sort();
            assert_eq!(table_ids(&defs, tier), expected, "tier {tier} pool");

            for (id, chance) in defs.chest_roll_table(tier) {
                let def = defs.get(&id).unwrap();
                let want = if def.chest_tier == Some(tier) {
                    def.chest_chance.unwrap_or(0.0)
                } else {
                    CHEST_CARRYOVER_CHANCE
                };
                assert_eq!(chance, want, "{id} chance at tier {tier}");
            }
        }

        // Each set's core debuts one dungeon above its opener.
        let debut = |id: &str| defs.get(id).unwrap().chest_tier;
        assert_eq!(debut("leather_helmet"), Some(1));
        assert_eq!(debut("leather_armor"), Some(2));
        assert_eq!(debut("chain_mail"), Some(3));
        assert_eq!(debut("breastplate"), Some(4));
        assert_eq!(debut("ring_of_protection"), Some(5));
        // Weapons and cash valuables stay out of chests entirely.
        for id in ["iron_sword", "gold_ring", "healing_potion"] {
            assert_eq!(debut(id), None, "{id} must stay out of chest pools");
        }
    }

    /// The doc's farming target: completing a tier's new set pieces takes
    /// ~5 chest opens on average. Closed form for independent per-open
    /// rolls — E[all K collected] by inclusion–exclusion over geometrics.
    #[test]
    fn chest_chances_hit_five_run_completion() {
        let defs = ItemDefs::load();

        fn expected_opens_to_collect(chances: &[f32]) -> f64 {
            let k = chances.len() as u32;
            let mut expectation = 0.0;
            for mask in 1..(1u32 << k) {
                let mut miss_all = 1.0;
                for (i, &p) in chances.iter().enumerate() {
                    if mask & (1 << i) != 0 {
                        miss_all *= 1.0 - f64::from(p);
                    }
                }
                let sign = if mask.count_ones() % 2 == 1 {
                    1.0
                } else {
                    -1.0
                };
                expectation += sign / (1.0 - miss_all);
            }
            expectation
        }

        // Old Crypt (tier 1): signature leather_helmet is guaranteed; the
        // remaining set pieces must land in ~5 runs.
        let t1: Vec<f32> = defs
            .chest_roll_table(1)
            .into_iter()
            .filter(|(id, _)| id != "leather_helmet")
            .map(|(_, chance)| chance)
            .collect();
        let opens = expected_opens_to_collect(&t1);
        assert!(
            (4.0..=6.0).contains(&opens),
            "old_crypt set completion expects ~5 opens, got {opens:.2}"
        );

        // Tier-2 home pieces roll at the doc's K=4 constant even while the
        // set is still missing assets; carryovers roll at the flat 10%.
        let t2: std::collections::HashMap<String, f32> =
            defs.chest_roll_table(2).into_iter().collect();
        assert_eq!(t2["iron_boots"], 0.37);
        assert_eq!(t2["iron_helmet"], 0.37);
        assert_eq!(t2["leather_gloves"], 0.37);
        assert_eq!(t2["leather_boots"], 0.37);
        assert_eq!(t2["raven_shield"], 0.2);
        assert_eq!(t2["leather_armor"], 0.0, "signature rolls only as itself");
        for id in ["leather_helmet", "leather_pants", "leather_belt"] {
            assert_eq!(t2[id], CHEST_CARRYOVER_CHANCE, "{id} carries over at 10%");
        }
    }

    #[test]
    fn catch_table_spans_fish_junk_and_coins() {
        let defs = ItemDefs::load();
        let table = defs.catch_table();
        let ids: Vec<&str> = table.iter().map(|c| c.item_def_id.as_str()).collect();
        for expected in [
            "raw_minnow",
            "golden_sturgeon",
            "old_boot",
            "message_in_a_bottle",
            "sunken_coin_pouch",
        ] {
            assert!(
                ids.contains(&expected),
                "{expected} missing from catch table"
            );
        }
        // Junk and coin catches are rarity 0: the XP formula (10·rarity²)
        // grants nothing for them, and only fish carry tiers ≥ 1.
        for c in table {
            let def = defs.get(&c.item_def_id).unwrap();
            if def.is_fish() {
                assert!(c.rarity >= 1, "{} fish tier", c.item_def_id);
            } else {
                assert_eq!(c.rarity, 0, "{} must be tier 0 (no XP)", c.item_def_id);
            }
        }
    }

    /// The economy guardrail as a contract test: the expected *sell* value of
    /// one catch must stay at coin-pile magnitude (the game's repeatable gold
    /// faucet is 1–10c piles; a catch should be worth a couple of piles, not
    /// a wage) — and it must hold at every fishing level, not just at level 0.
    /// Averaging over raw `catchWeight` would only ever measure a beginner.
    #[test]
    fn expected_catch_value_stays_in_the_coin_pile_economy_at_every_level() {
        fn dice_avg(notation: &str) -> f64 {
            let (n, m) = notation.split_once('d').expect("NdM");
            let n: f64 = n.parse().unwrap();
            let m: f64 = m.parse().unwrap();
            n * (m + 1.0) / 2.0
        }
        let defs = ItemDefs::load();
        let table = defs.catch_table();
        let sell_rate = crate::merchant_defs::merchant_defs()
            .get_by_npc_name("Rica")
            .expect("Rica has a merchant definition")
            .sell_rate_percent as f64
            / 100.0;
        let ev_at = |level: u32| -> f64 {
            let weights = crate::game_state::fishing::effective_weights(table, level);
            let total: f64 = weights.iter().map(|w| *w as f64).sum();
            weights
                .iter()
                .zip(table)
                .map(|(weight, c)| {
                    let def = defs.get(&c.item_def_id).unwrap();
                    let value = if def.is_coin_catch() {
                        // Coins arrive at face value.
                        def.dice.as_deref().map_or(0.0, dice_avg)
                    } else {
                        // Items sell at Rica's merchant rate.
                        def.base_price.unwrap_or(0) as f64 * sell_rate
                    };
                    *weight as f64 * value
                })
                .sum::<f64>()
                / total
        };

        let evs: Vec<f64> = (0..=SKILL_LEVEL_CAP).map(ev_at).collect();
        for (level, ev) in evs.iter().enumerate() {
            assert!(
                (5.0..=25.0).contains(ev),
                "expected sell value per catch at level {level} is {ev:.1}c — outside \
                 the 5–25c coin-pile band"
            );
        }
        assert!(
            evs.windows(2).all(|w| w[1] >= w[0]),
            "skill should never make an angler poorer"
        );
        // Mastery pays a better wage, not a different economy. Without this
        // the old additive weighting reached 10x and no test noticed.
        assert!(
            evs[evs.len() - 1] <= 4.0 * evs[0],
            "level {SKILL_LEVEL_CAP} earns {:.1}c vs {:.1}c at level 0 — that is a \
             different economy, not a better wage",
            evs[evs.len() - 1],
            evs[0]
        );
    }

    /// The flotsam price sheet: gag junk is worthless by design, the bottle
    /// pays a token, and the pouch pays through its dice — not a resale price.
    #[test]
    fn junk_pricing_matches_the_gag() {
        let defs = ItemDefs::load();
        assert!(
            defs.get("old_boot").unwrap().base_price.is_none(),
            "a boot is worthless by design"
        );
        assert!(defs.get("clump_of_kelp").unwrap().base_price.is_none());
        assert_eq!(
            defs.get("message_in_a_bottle").unwrap().base_price,
            Some(15)
        );
        let pouch = defs.get("sunken_coin_pouch").unwrap();
        assert!(pouch.is_coin_catch());
        assert_eq!(pouch.dice.as_deref(), Some("3d8"));
        assert!(
            pouch.base_price.is_none(),
            "the pouch pays via its dice, not a merchant sale"
        );
    }

    /// Trophies are gated to fish: junk never celebrates, a natural 20 always
    /// does on a fish, and the size threshold is an exact boundary.
    #[test]
    fn trophies_are_a_fish_concept() {
        let defs = ItemDefs::load();
        let boot = defs.get("old_boot").unwrap();
        assert!(
            !boot.trophy_at(200, true),
            "a nat-20 boot is still just a boot"
        );
        let minnow = defs.get("raw_minnow").unwrap();
        assert!(
            minnow.trophy_at(1, true),
            "a natural 20 is always a trophy on a fish"
        );
        let trout = defs.get("raw_trout").unwrap();
        let threshold = trout.trophy_cm.unwrap() as u16;
        assert!(trout.trophy_at(threshold, false));
        assert!(!trout.trophy_at(threshold - 1, false));
    }

    #[test]
    fn fishing_rod_is_a_rod_not_a_weapon() {
        let defs = ItemDefs::load();
        let rod = defs.get("fishing_rod").expect("fishing_rod def");
        assert!(rod.is_fishing_rod());
        assert!(!rod.is_weapon(), "the rod must not deal weapon damage");
    }
}

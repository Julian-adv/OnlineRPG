use crate::auth::{AuthService, ItemRow};
use crate::item_defs::UseEffect;
use crate::types::{PlayerId, ServerMessage};
use onlinerpg_shared::inventory::{EquipSlot, GroundItem, ItemInstance, PlayerInventory};
use onlinerpg_shared::messages::BagLineItem;
use rand::Rng;
use tracing::{info, warn};

use super::party::SummonCast;
use super::ServerGroundItem;

/// Ground items despawn after 30 minutes — long enough that a tip dropped at
/// the start of the longest song (~5 min) still lies there through the rest
/// and the thanks after it.
const GROUND_ITEM_LIFETIME_MS: u64 = 30 * 60 * 1000;

const MAX_PICKUP_DISTANCE: f32 = 2.5;

pub(super) const PLACEMENT_DISTANCE_M: f32 = 1.0;

/// Enchant odds are expressed in basis points (1/100 of a percent) out of
/// this scale; the handler's roll must use the same bound.
const ENCHANT_BP_SCALE: u32 = 10_000;

/// What one enchant scroll does: how it finds its target, its odds ladder,
/// and the lines it prints. `read_enchant_scroll` owns everything else.
struct EnchantScroll {
    /// Picks the slot to enchant, given a pre-rolled `pick` to choose among
    /// equally valid targets. `None` refuses the read.
    select: fn(&PlayerInventory, &crate::item_defs::ItemDefs, u64) -> Option<EquipSlot>,
    ladder: fn(i32) -> u32,
    no_target: &'static str,
    destroyed: fn(&str) -> String,
    honed: fn(&str, i32) -> String,
}

/// Success chance, in basis points, of enchanting an item currently at
/// `enchant`. Guaranteed through +4, then the over-enchanting gamble:
/// 75/50/25% at +5/+6/+7, halving each level from +8 until the 1% floor at
/// +12 — the ladder never closes entirely, it just gets very expensive.
fn enchant_success_bp(enchant: i32) -> u32 {
    match enchant {
        ..=4 => ENCHANT_BP_SCALE,
        5 => 7_500,
        6 => 5_000,
        7 => 2_500,
        8 => 1_250,
        9 => 625,
        10 => 312,
        11 => 156,
        _ => 100, // the 1% floor
    }
}

/// The ladder shifted two levels down for armor — free only through +2, since
/// armor fills six slots to a weapon's one (doc/ENCHANT.md).
fn armor_enchant_success_bp(enchant: i32) -> u32 {
    enchant_success_bp(enchant.saturating_add(2))
}

/// A client-supplied cloth colour as `#rrggbb`, lowercased so the change
/// compare in `set_player_gear` and the clients' material cache both key on
/// one spelling. `None` if it is not a hex colour.
pub(super) fn normalize_hex_color(color: &str) -> Option<String> {
    let hex = color.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_ascii_lowercase()))
}

/// One unit-insert request: `quantity` units of one def at one enchant level,
/// backed by ids starting at `first_instance_id`.
pub(super) struct BagInsert<'a> {
    pub stackable: bool,
    pub item_def_id: &'a str,
    pub enchant: i32,
    /// Dye carried by the units being inserted, so a dyed cape stays dyed
    /// through a pickup, a trade or a buyback.
    pub cape_color: Option<String>,
    /// Texture hash carried the same way.
    pub cape_texture: Option<String>,
    pub first_instance_id: u64,
    pub quantity: u32,
}

impl<'a> BagInsert<'a> {
    pub(super) fn one(
        stackable: bool,
        item_def_id: &'a str,
        enchant: i32,
        first_instance_id: u64,
    ) -> Self {
        Self {
            stackable,
            item_def_id,
            enchant,
            first_instance_id,
            quantity: 1,
            cape_color: None,
            cape_texture: None,
        }
    }

    pub(super) fn with_cape_skin(
        mut self,
        cape_color: Option<String>,
        cape_texture: Option<String>,
    ) -> Self {
        self.cape_color = cape_color;
        self.cape_texture = cape_texture;
        self
    }
}

/// The one bag-insert rule for every acquisition path (buying, loot, grilling,
/// pickup, login load): stackable units join an existing same-def, same-enchant
/// entry, non-stackables take one slot per unit off consecutive ids. Returns
/// how many ids were consumed, so batch callers can advance a reserved range.
/// Equip/unequip move an existing instance instead and stay outside this rule,
/// which `ItemDefs::load`'s stackable-vs-equippable assert keeps safe.
pub(super) fn stack_into_bag(bag: &mut Vec<ItemInstance>, insert: BagInsert) -> u64 {
    let BagInsert {
        stackable,
        item_def_id,
        enchant,
        cape_color,
        cape_texture,
        first_instance_id,
        quantity,
    } = insert;
    if quantity == 0 {
        return 0;
    }

    if stackable {
        if let Some(stack) = bag.iter_mut().find(|item| {
            item.item_def_id == item_def_id
                && item.enchant == enchant
                && item.cape_color == cape_color
                && item.cape_texture == cape_texture
        }) {
            stack.quantity += quantity;
            return 0;
        }
        bag.push(ItemInstance {
            instance_id: first_instance_id,
            item_def_id: item_def_id.to_string(),
            quantity,
            enchant,
            cape_color,
            cape_texture,
        });
        return 1;
    }

    for offset in 0..quantity as u64 {
        bag.push(ItemInstance {
            instance_id: first_instance_id + offset,
            item_def_id: item_def_id.to_string(),
            quantity: 1,
            enchant,
            cape_color: cape_color.clone(),
            cape_texture: cape_texture.clone(),
        });
    }
    quantity as u64
}

/// One draw out of a bag: units that left together because they shared an
/// entry, and the per-instance state they carry with them.
#[derive(Default)]
pub(super) struct Draw {
    pub enchant: i32,
    pub cape_color: Option<String>,
    pub cape_texture: Option<String>,
    pub quantity: u32,
}

/// The one bag-draw rule for def-keyed stock sales: requests carry no enchant,
/// so units leave lowest-enchant-first. Stops short if the bag runs out.
pub(super) fn draw_from_bag(
    bag: &mut Vec<ItemInstance>,
    item_def_id: &str,
    mut quantity: u32,
) -> Vec<Draw> {
    let mut draws = Vec::new();
    while quantity > 0 {
        let Some(idx) = bag
            .iter()
            .enumerate()
            .filter(|(_, item)| item.item_def_id == item_def_id)
            .min_by_key(|(_, item)| item.enchant)
            .map(|(idx, _)| idx)
        else {
            break;
        };
        let take = quantity.min(bag[idx].quantity);
        draws.push(Draw {
            enchant: bag[idx].enchant,
            cape_color: bag[idx].cape_color.clone(),
            cape_texture: bag[idx].cape_texture.clone(),
            quantity: take,
        });
        if bag[idx].quantity > take {
            bag[idx].quantity -= take;
        } else {
            bag.remove(idx);
        }
        quantity -= take;
    }
    draws
}

/// One consumable that changes the worn cape: how to recognise it in a bag,
/// and the three lines it says. Dye and print are the two of these
/// (doc/CAPE_CUSTOMIZATION.md); everything else about them is shared.
struct CapeTool {
    recognize: fn(&UseEffect) -> bool,
    defeated: &'static str,
    no_cape: &'static str,
    done: &'static str,
}

const CAPE_DYE: CapeTool = CapeTool {
    recognize: |effect| matches!(effect, UseEffect::PromptCapeDye),
    defeated: "You can't dye while defeated",
    no_cape: "You are not wearing a cape to dye",
    done: "The dye soaks into the cloth.",
};

const CAPE_PRINT: CapeTool = CapeTool {
    recognize: |effect| matches!(effect, UseEffect::PromptCapeTexture),
    defeated: "You can't do that while defeated",
    no_cape: "You are not wearing a cape to print on",
    done: "The print takes to the cloth.",
};

/// The reagent every enchant reading burns, on top of the scroll
/// (doc/ENCHANT.md). Bought from Rica; it is the enchant ladder's gold sink.
const WHETSTONE_OIL_ITEM_ID: &str = "whetstone_oil";

/// Remove one unit of `instance_id` from the bag, dropping the instance when
/// the stack empties.
/// Returns the removed unit's def id, so callers can log without rescanning.
fn consume_one(inv: &mut PlayerInventory, instance_id: u64) -> Option<String> {
    let idx = inv.bag.iter().position(|i| i.instance_id == instance_id)?;
    let def_id = inv.bag[idx].item_def_id.clone();
    if inv.bag[idx].quantity > 1 {
        inv.bag[idx].quantity -= 1;
    } else {
        inv.bag.remove(idx);
    }
    Some(def_id)
}

/// Serialize a PlayerInventory into the flat row format used by AuthService
/// persistence.
/// Where a hand-dropped item lands: a step in front of the player plus a
/// small scatter, so a run of drops doesn't pile onto one pixel.
fn drop_landing_position(origin: crate::types::Position, rotation: f32) -> crate::types::Position {
    let (landing_angle, landing_distance) = {
        let mut rng = rand::thread_rng();
        (
            rng.gen::<f32>() * std::f32::consts::TAU,
            rng.gen::<f32>().sqrt() * 0.7,
        )
    };
    crate::types::Position {
        x: origin.x + rotation.sin() + landing_angle.cos() * landing_distance,
        y: origin.y,
        z: origin.z + rotation.cos() + landing_angle.sin() * landing_distance,
    }
}

pub(super) fn serialize_inventory(inv: &PlayerInventory) -> Vec<ItemRow> {
    let mut rows: Vec<ItemRow> = inv
        .bag
        .iter()
        .map(|item| ItemRow {
            item_def_id: item.item_def_id.clone(),
            quantity: item.quantity,
            equip_slot: None,
            enchant: item.enchant,
            cape_color: item.cape_color.clone(),
            cape_texture: item.cape_texture.clone(),
        })
        .collect();
    for (slot, item) in &inv.equipped {
        rows.push(ItemRow {
            item_def_id: item.item_def_id.clone(),
            quantity: 1,
            equip_slot: Some(slot.as_str().to_string()),
            enchant: item.enchant,
            cape_color: item.cape_color.clone(),
            cape_texture: item.cape_texture.clone(),
        });
    }
    rows
}

impl super::GameState {
    /// Reserve a range of instance IDs (single lock acquisition).
    pub(super) async fn reserve_instance_ids(&self, count: u64) -> u64 {
        let mut id = self.next_item_instance_id.write().await;
        let start = *id;
        *id += count;
        start
    }

    pub(super) async fn next_instance_id(&self) -> u64 {
        self.reserve_instance_ids(1).await
    }

    /// D&D 5e carry weight: STR * 15, scaled by the hunger band
    /// (doc/HUNGER.md). Dropping below a cap only refuses new pickups and
    /// purchases — it never blocks movement. Reads `player_characters` and
    /// `hunger`, which rank below `player_gold`/`inventories`: call it before
    /// taking those, never under them.
    pub(super) async fn max_carry_weight(&self, player_id: &PlayerId) -> f32 {
        let base = {
            let chars = self.player_characters.read().await;
            if let Some((_, _, attrs)) = chars.get(player_id) {
                attrs.r#str as f32 * 15.0
            } else {
                150.0
            }
        };
        base * self.hunger_carry_mult(player_id).await
    }

    /// `armor_mult` soaks the armour, worn and packed alike (doc/DEBUFF.md);
    /// the caller reads it before taking `inventories`.
    pub(super) fn calc_total_weight(&self, inventory: &PlayerInventory, armor_mult: f32) -> f32 {
        let weigh = |item: &ItemInstance| self.item_defs.weight_with(&item.item_def_id, armor_mult);
        let bag_weight: f32 = inventory
            .bag
            .iter()
            .map(|item| weigh(item) * item.quantity as f32)
            .sum();
        let equip_weight: f32 = inventory.equipped.values().map(weigh).sum();
        bag_weight + equip_weight
    }

    /// Send the current inventory state directly to a player, then their
    /// refreshed effective stats. Every equipped-gear mutation routes through
    /// here so no mutation site has to remember to send them.
    pub(super) async fn send_inventory_snapshot(
        &self,
        player_id: &PlayerId,
        inventory: PlayerInventory,
    ) {
        self.set_player_gear(player_id, &inventory).await;
        self.refresh_hunger_gear_drain(player_id, &inventory).await;
        self.send_direct_message(player_id, ServerMessage::InventoryUpdated { inventory })
            .await;
        let stats = self.effective_stats(player_id).await;
        self.send_direct_message(player_id, stats.into()).await;
    }

    /// Load a player's inventory from the database into memory.
    pub async fn load_player_inventory(
        &self,
        player_id: &PlayerId,
        character_id: i64,
        auth: &AuthService,
    ) {
        let auth = auth.clone();
        let loaded = tokio::task::spawn_blocking(move || auth.load_inventory(character_id))
            .await
            .unwrap_or_else(|e| {
                warn!("spawn_blocking panicked loading inventory: {}", e);
                Err(crate::auth::AuthError::Database(e.to_string()))
            });

        let rows = match loaded {
            Ok(data) => data,
            Err(e) => {
                warn!(
                    "Failed to load inventory for character {}: {}",
                    character_id, e
                );
                return;
            }
        };

        let mut inventory = PlayerInventory::default();

        if !rows.is_empty() {
            // A non-stackable row saved with quantity N unfolds into N slots,
            // so reserve per unit rather than per row.
            let units: u64 = rows.iter().map(|r| r.quantity.max(1) as u64).sum();
            let mut next_id = self.reserve_instance_ids(units).await;

            for row in rows {
                match row.equip_slot {
                    Some(slot_str) => {
                        if let Ok(slot) = slot_str.parse::<EquipSlot>() {
                            inventory.equipped.insert(
                                slot,
                                ItemInstance {
                                    instance_id: next_id,
                                    item_def_id: row.item_def_id,
                                    quantity: 1,
                                    enchant: row.enchant,
                                    cape_color: row.cape_color,
                                    cape_texture: row.cape_texture,
                                },
                            );
                            next_id += 1;
                        } else {
                            warn!(
                                "Unknown equip slot '{}' in DB for character {}",
                                slot_str, character_id
                            );
                        }
                    }
                    None => {
                        // Merging here also heals bags saved before trading
                        // and pickup learned to stack.
                        next_id += stack_into_bag(
                            &mut inventory.bag,
                            BagInsert {
                                stackable: self.item_defs.stackable(&row.item_def_id),
                                item_def_id: &row.item_def_id,
                                enchant: row.enchant,
                                cape_color: row.cape_color,
                                cape_texture: row.cape_texture,
                                first_instance_id: next_id,
                                quantity: row.quantity,
                            },
                        );
                    }
                }
            }
        }

        self.refresh_hunger_gear_drain(player_id, &inventory).await;
        self.inventories.write().await.insert(*player_id, inventory);
    }

    /// Detach a player's inventory from memory and hand back the snapshot to
    /// persist. The character id is resolved *before* the removal, so a missing
    /// mapping bails without dropping the inventory; the `remove` then captures
    /// the items in one step, stopping a departing session from mutating them
    /// between the read and the detach (F-015).
    pub async fn take_player_inventory(&self, player_id: &PlayerId) -> Option<(i64, Vec<ItemRow>)> {
        let char_id = {
            let player_chars = self.player_characters.read().await;
            let (char_id, _, _) = player_chars.get(player_id)?;
            *char_id
        };
        let removed = {
            let mut inventories = self.inventories.write().await;
            inventories.remove(player_id)
        };
        {
            let mut dirty = self.dirty_inventories.write().await;
            dirty.remove(player_id);
        }

        Some((char_id, serialize_inventory(&removed?)))
    }

    pub async fn get_player_inventory(&self, player_id: &PlayerId) -> Option<PlayerInventory> {
        let inventories = self.inventories.read().await;
        inventories.get(player_id).cloned()
    }

    /// Push the player's current inventory, for mutations that happen behind
    /// their own locks and have no snapshot in hand.
    pub(super) async fn push_inventory_update(&self, player_id: &PlayerId) {
        let snapshot = self.inventories.read().await.get(player_id).cloned();
        if let Some(snapshot) = snapshot {
            self.send_inventory_snapshot(player_id, snapshot).await;
        }
    }

    pub(super) async fn mark_inventory_dirty(&self, player_id: &PlayerId) {
        let mut dirty = self.dirty_inventories.write().await;
        dirty.insert(*player_id);
    }

    pub(super) async fn restore_dirty_inventories(&self, ids: Vec<PlayerId>) {
        if !ids.is_empty() {
            self.dirty_inventories.write().await.extend(ids);
        }
    }

    pub async fn give_item(&self, player_id: &PlayerId, item_def_id: &str) -> bool {
        let Some(stackable) = self.item_defs.get(item_def_id).map(|d| d.stackable) else {
            warn!("give_item: unknown item_def_id {:?}", item_def_id);
            return false;
        };

        let instance_id = self.next_instance_id().await;
        let snapshot = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return false,
            };
            stack_into_bag(
                &mut inv.bag,
                BagInsert::one(stackable, item_def_id, 0, instance_id),
            );
            inv.clone()
        };

        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        true
    }

    /// Award one unit of an item, respecting the carry-weight cap: stacks onto
    /// an existing bag entry when the def says `stackable` (otherwise each unit
    /// takes its own slot), and when the unit would not fit, spills it to the
    /// ground at the player's feet instead — an award is never silently lost.
    /// Fishing catches land here.
    pub async fn award_item(&self, player_id: &PlayerId, item_def_id: &str) {
        let Some(stackable) = self.item_defs.get(item_def_id).map(|d| d.stackable) else {
            warn!("award_item: unknown item_def_id {:?}", item_def_id);
            return;
        };
        let max_weight = self.max_carry_weight(player_id).await;
        let armor_mult = self.armor_weight_mult(player_id).await;
        let def_weight = self.item_defs.weight_with(item_def_id, armor_mult);
        // Reserved before the inventory lock; unused when the unit stacks
        // onto an existing entry. A skipped id is cheaper than lock nesting.
        let reserved_instance_id = self.next_instance_id().await;

        enum Placement {
            Bagged(PlayerInventory),
            Overweight,
        }
        let placement = {
            let mut inventories = self.inventories.write().await;
            let Some(inv) = inventories.get_mut(player_id) else {
                return;
            };
            if self.calc_total_weight(inv, armor_mult) + def_weight > max_weight {
                Placement::Overweight
            } else {
                stack_into_bag(
                    &mut inv.bag,
                    BagInsert::one(stackable, item_def_id, 0, reserved_instance_id),
                );
                Placement::Bagged(inv.clone())
            }
        };

        match placement {
            Placement::Bagged(snapshot) => {
                self.mark_inventory_dirty(player_id).await;
                self.send_inventory_snapshot(player_id, snapshot).await;
            }
            Placement::Overweight => {
                let (position, floor_level) = {
                    let players = self.players.read().await;
                    match players.get(player_id) {
                        Some(p) => (p.position, p.floor_level),
                        None => return,
                    }
                };
                self.send_system_message(player_id, "Too heavy to carry — it slips to the ground.")
                    .await;
                self.spawn_ground_item(GroundItem {
                    instance_id: reserved_instance_id,
                    item_def_id: item_def_id.to_string(),
                    position,
                    floor_level,
                    quantity: 1,
                    enchant: 0,
                    dropped_by: Some(*player_id),
                    cape_color: None,
                    cape_texture: None,
                })
                .await;
            }
        }
    }

    /// Whether the wielded main-hand weapon claims both hands, sealing the
    /// off-hand slot.
    fn main_hand_is_two_handed(&self, inv: &PlayerInventory) -> bool {
        inv.equipped
            .get(&EquipSlot::MainHand)
            .and_then(|item| self.item_defs.get(&item.item_def_id))
            .is_some_and(|def| def.is_two_handed())
    }

    pub async fn equip_item(&self, player_id: &PlayerId, instance_id: u64) {
        if self
            .reject_if_trade_reserved(player_id, instance_id, "equip")
            .await
        {
            return;
        }
        let (snapshot, torch_on) = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return,
            };

            let bag_idx = match inv.bag.iter().position(|i| i.instance_id == instance_id) {
                Some(idx) => idx,
                None => {
                    drop(inventories);
                    self.send_system_message(player_id, "Item not found in bag")
                        .await;
                    return;
                }
            };

            let item_def_id = inv.bag[bag_idx].item_def_id.clone();
            let equip_slot = match self.item_defs.get(&item_def_id).and_then(|d| d.equip_slot) {
                Some(slot) => slot,
                None => {
                    drop(inventories);
                    self.send_system_message(player_id, "This item cannot be equipped")
                        .await;
                    return;
                }
            };

            // Two-handed weapons own the off-hand slot: equipping one empties
            // it, and nothing may move back in while it is wielded.
            if equip_slot == EquipSlot::OffHand && self.main_hand_is_two_handed(inv) {
                drop(inventories);
                self.send_system_message(player_id, "Both hands are on your weapon")
                    .await;
                return;
            }

            let target_slot = if inv.equipped.contains_key(&equip_slot) {
                equip_slot
                    .alternate()
                    .filter(|alt| !inv.equipped.contains_key(alt))
                    .unwrap_or(equip_slot)
            } else {
                equip_slot
            };

            let item = inv.bag.remove(bag_idx);
            let two_handed = self
                .item_defs
                .get(&item.item_def_id)
                .is_some_and(|def| def.is_two_handed());
            if let Some(old_item) = inv.equipped.insert(target_slot, item) {
                inv.bag.push(old_item);
            }
            let mut off_hand_cleared = false;
            if two_handed && target_slot == EquipSlot::MainHand {
                if let Some(displaced) = inv.equipped.remove(&EquipSlot::OffHand) {
                    inv.bag.push(displaced);
                    off_hand_cleared = true;
                }
            }
            let torch_on =
                (target_slot == EquipSlot::OffHand || off_hand_cleared).then(|| inv.is_torch_lit());
            (inv.clone(), torch_on)
        };

        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        if let Some(torch_on) = torch_on {
            self.set_player_torch(player_id, torch_on).await;
        }
        self.abort_fishing_if_rod_lost(player_id).await;
    }

    pub async fn unequip_item(&self, player_id: &PlayerId, slot: EquipSlot) {
        let snapshot = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return,
            };

            match inv.equipped.remove(&slot) {
                Some(item) => {
                    inv.bag.push(item);
                    inv.clone()
                }
                None => {
                    drop(inventories);
                    self.send_system_message(player_id, "No item in that slot")
                        .await;
                    return;
                }
            }
        };

        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        if slot == EquipSlot::OffHand {
            self.set_player_torch(player_id, false).await;
        }
        self.abort_fishing_if_rod_lost(player_id).await;
    }

    /// Use a consumable from the bag: resolve its effect and dispatch to the
    /// matching handler (healing potion, return scroll, ...).
    pub async fn use_item(&self, player_id: &PlayerId, instance_id: u64) {
        if self
            .reject_if_trade_reserved(player_id, instance_id, "use")
            .await
        {
            return;
        }
        // Resolve which usable effect this item carries before mutating anything.
        let effect = {
            let inventories = self.inventories.read().await;
            let inv = match inventories.get(player_id) {
                Some(inv) => inv,
                None => return,
            };
            let item = match inv.bag.iter().find(|i| i.instance_id == instance_id) {
                Some(item) => item,
                None => {
                    drop(inventories);
                    self.send_system_message(player_id, "Item not found in bag")
                        .await;
                    return;
                }
            };
            let effect = self
                .item_defs
                .get(&item.item_def_id)
                .and_then(|def| def.use_effect());
            match effect {
                Some(effect) => effect,
                None => {
                    drop(inventories);
                    self.send_system_message(player_id, "This item cannot be used")
                        .await;
                    return;
                }
            }
        };

        match effect {
            UseEffect::Heal(dice) => self.use_healing_item(player_id, instance_id, &dice).await,
            UseEffect::Eat(eat) => self.use_eat_item(player_id, instance_id, &eat, None).await,
            UseEffect::PlaceCampfire => self.use_campfire_kit(player_id, instance_id).await,
            UseEffect::TeleportTown => self.use_return_scroll(player_id, instance_id).await,
            UseEffect::EnchantWeapon => {
                self.use_enchant_weapon_scroll(player_id, instance_id).await
            }
            UseEffect::EnchantArmor => self.use_enchant_armor_scroll(player_id, instance_id).await,
            UseEffect::SummonParty => self.use_party_summon_scroll(player_id, instance_id).await,
            UseEffect::OpenCoinPouch(dice) => {
                self.use_coin_pouch(player_id, instance_id, &dice).await
            }
            UseEffect::ToggleTipHat => self.toggle_tip_hat(player_id).await,
            UseEffect::PromptCapeDye => {
                self.prompt_cape_tool(
                    player_id,
                    &CAPE_DYE,
                    ServerMessage::CapeDyePrompt { instance_id },
                )
                .await
            }
            UseEffect::PromptCapeTexture => {
                self.prompt_cape_tool(
                    player_id,
                    &CAPE_PRINT,
                    ServerMessage::CapeTexturePrompt { instance_id },
                )
                .await
            }
            UseEffect::ReviveInPlace(hp_percent) => {
                self.use_phoenix_talisman(player_id, instance_id, hp_percent)
                    .await
            }
        }
    }

    /// Use a phoenix talisman: revive a defeated user where they fell. The
    /// revive runs first and only a successful one spends the talisman, so
    /// using it alive — or twice — keeps it.
    async fn use_phoenix_talisman(&self, player_id: &PlayerId, instance_id: u64, hp_percent: u32) {
        if !self.revive_in_place(player_id, hp_percent).await {
            self.send_system_message(player_id, "The talisman only stirs for the fallen")
                .await;
            return;
        }
        self.consume_one_and_sync(player_id, instance_id).await;
    }

    /// Open the client's picker, spending nothing. Refuses here rather than
    /// after the pick so nobody chooses a colour or a picture only to be told
    /// they have no cape on.
    async fn prompt_cape_tool(&self, player_id: &PlayerId, tool: &CapeTool, prompt: ServerMessage) {
        if self.reject_if_defeated(player_id, tool.defeated).await {
            return;
        }
        let wearing = {
            let inventories = self.inventories.read().await;
            inventories
                .get(player_id)
                .is_some_and(|inv| self.wears_cape(inv))
        };
        if !wearing {
            self.send_system_message(player_id, tool.no_cape).await;
            return;
        }
        self.send_direct_message(player_id, prompt).await;
    }

    /// Whether the back slot holds something dyeable. One spelling for both
    /// the prompt (read lock) and the dye itself (write lock), so the two
    /// cannot drift into disagreeing about what a cape is.
    fn wears_cape(&self, inv: &PlayerInventory) -> bool {
        inv.equipped
            .get(&EquipSlot::Back)
            .and_then(|item| self.item_defs.get(&item.item_def_id))
            .is_some_and(|def| def.is_cape())
    }

    /// Spend `instance_id` and let `apply` change the worn cape. Everything
    /// the prompt checked is checked again: the round trip through the picker
    /// is unbounded, and the cape can come off or the tool be traded away in
    /// between. One body for every cape tool, because these are the security
    /// checks — a copy per tool is a copy that drifts.
    async fn change_worn_cape(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        tool: &CapeTool,
        apply: impl FnOnce(&mut ItemInstance),
    ) {
        if self
            .reject_if_trade_reserved(player_id, instance_id, "use")
            .await
        {
            return;
        }
        if self.reject_if_defeated(player_id, tool.defeated).await {
            return;
        }

        let snapshot = {
            let mut inventories = self.inventories.write().await;
            let Some(inv) = inventories.get_mut(player_id) else {
                return;
            };
            let holds_tool = inv
                .bag
                .iter()
                .find(|item| item.instance_id == instance_id)
                .and_then(|item| self.item_defs.get(&item.item_def_id))
                .and_then(|def| def.use_effect())
                .is_some_and(|effect| (tool.recognize)(&effect));
            if !holds_tool {
                drop(inventories);
                self.send_system_message(player_id, "Item not found in bag")
                    .await;
                return;
            }
            if !self.wears_cape(inv) {
                drop(inventories);
                self.send_system_message(player_id, tool.no_cape).await;
                return;
            }
            apply(
                inv.equipped
                    .get_mut(&EquipSlot::Back)
                    .expect("checked above"),
            );
            consume_one(inv, instance_id);
            inv.clone()
        };

        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        self.send_system_message(player_id, tool.done).await;
    }

    /// Dye the worn cape and spend the dye. The colour is checked first: it is
    /// the one check that reads no state.
    pub async fn dye_cape(&self, player_id: &PlayerId, instance_id: u64, color: &str) {
        let Some(color) = normalize_hex_color(color) else {
            self.send_system_message(player_id, "That is not a colour")
                .await;
            return;
        };
        self.change_worn_cape(player_id, instance_id, &CAPE_DYE, |cape| {
            cape.cape_color = Some(color)
        })
        .await;
    }

    /// Put an already-uploaded texture on the worn cape and spend the kit.
    /// `texture` is a content hash the upload endpoint handed back; it is
    /// checked against the store rather than trusted, or a client could name
    /// any string and every nearby client would turn it into a URL.
    pub async fn apply_cape_texture(&self, player_id: &PlayerId, instance_id: u64, texture: &str) {
        if !self.cape_textures.is_wearable(texture).await {
            self.send_system_message(player_id, "That picture is no longer available")
                .await;
            return;
        }
        self.change_worn_cape(player_id, instance_id, &CAPE_PRINT, |cape| {
            cape.cape_texture = Some(texture.to_string())
        })
        .await;
    }

    /// File a complaint about the texture another player is wearing. Nothing
    /// is hidden on report — an admin blocks the hash, which unwears it
    /// everywhere at once.
    pub async fn report_cape_texture(&self, reporter_id: &PlayerId, target_id: &PlayerId) {
        let (hash, target_name) = {
            let players = self.players.read().await;
            match players.get(target_id) {
                Some(target) => (target.back_texture.clone(), target.name.clone()),
                None => (None, String::new()),
            }
        };
        let Some(hash) = hash else {
            self.send_system_message(reporter_id, "They are not wearing a printed cape")
                .await;
            return;
        };
        let reporter_name = self.player_name_of(reporter_id).await;
        self.cape_textures
            .record_report(&hash, &reporter_name, &target_name)
            .await;
        self.send_system_message(reporter_id, "Reported. Thank you.")
            .await;
    }

    /// Open a fished-up coin pouch: roll its dice for the copper inside,
    /// spend the pouch, and credit the wallet. The system line puts the
    /// amount in the combat log; `award_copper` drives the gold popup.
    async fn use_coin_pouch(&self, player_id: &PlayerId, instance_id: u64, dice: &str) {
        let name = {
            let inventories = self.inventories.read().await;
            let def_id = inventories
                .get(player_id)
                .and_then(|inv| inv.bag.iter().find(|i| i.instance_id == instance_id))
                .map(|item| item.item_def_id.clone());
            match def_id {
                Some(def_id) => self.item_name(&def_id),
                // The pouch raced away since `use_item` resolved the effect.
                None => return,
            }
        };
        let copper = crate::game::combat::roll_dice(dice);
        self.consume_one_and_sync(player_id, instance_id).await;
        self.award_copper(player_id, i64::from(copper)).await;
        info!(
            "Player {} opened a coin pouch: +{} copper",
            self.player_name_of(player_id).await,
            copper
        );
        self.send_system_message(
            player_id,
            format!("You open the {name} — {copper} copper spills out."),
        )
        .await;
    }

    /// Drink a healing potion: roll its dice and restore HP up to the cap.
    /// Refuses (keeping the potion) if the player is defeated or already full.
    async fn use_healing_item(&self, player_id: &PlayerId, instance_id: u64, heal_dice: &str) {
        {
            let players = self.players.read().await;
            let Some(player) = players.get(player_id) else {
                return;
            };
            if player.health == 0 {
                drop(players);
                self.send_system_message(player_id, "You can't drink while defeated")
                    .await;
                return;
            }
            if player.health >= player.max_health {
                drop(players);
                self.send_system_message(player_id, "You are already at full health")
                    .await;
                return;
            }
        }

        self.consume_one_and_sync(player_id, instance_id).await;
        self.roll_heal_and_broadcast(player_id, heal_dice).await;
    }

    /// Eat food or fish; raw fish near a campfire grills instead.
    /// `force_debuff` pins the `debuff` roll for tests.
    pub(super) async fn use_eat_item(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        eat: &crate::item_defs::EatEffect,
        force_debuff: Option<bool>,
    ) {
        let crate::item_defs::EatEffect {
            nutrition,
            raw_fish,
            debuff,
            alcohol,
        } = eat;
        let (nutrition, raw_fish, alcohol) = (*nutrition, *raw_fish, *alcohol);
        if self
            .reject_if_defeated(player_id, "You can't eat while defeated")
            .await
        {
            return;
        }

        let (def_id, position, floor_level) = {
            let inventories = self.inventories.read().await;
            let def_id = match inventories
                .get(player_id)
                .and_then(|inv| inv.bag.iter().find(|i| i.instance_id == instance_id))
            {
                Some(item) => item.item_def_id.clone(),
                None => return,
            };
            drop(inventories);
            let players = self.players.read().await;
            let Some(p) = players.get(player_id) else {
                return;
            };
            (def_id, p.position, p.floor_level)
        };

        if raw_fish
            && self
                .try_start_grill(player_id, instance_id, &def_id, &position, floor_level)
                .await
        {
            return;
        }

        let (outcome, gained) = self.apply_eat(player_id, nutrition).await;
        // A meal that adds no satiation (already full) is refused outright —
        // nothing has been consumed yet.
        if gained == 0 {
            self.send_system_message(player_id, "You are too full to eat another bite")
                .await;
            return;
        }
        self.consume_one_and_sync(player_id, instance_id).await;
        let name = self.item_name(&def_id);
        self.send_system_message(player_id, format!("You eat the {name}."))
            .await;
        self.settle_meal(player_id, outcome, gained).await;
        if let Some(debuff) = debuff {
            self.inflict_debuff(player_id, debuff, force_debuff).await;
        }
        if let Some(units) = alcohol {
            self.apply_alcohol(player_id, units).await;
        }
    }

    /// Use a campfire kit outdoors and out of standing water.
    async fn use_campfire_kit(&self, player_id: &PlayerId, instance_id: u64) {
        if self
            .reject_if_defeated(player_id, "You can't build a fire while defeated")
            .await
        {
            return;
        }
        let Some((placement, floor_level)) = self.campfire_placement(player_id).await else {
            return;
        };
        self.consume_one_and_sync(player_id, instance_id).await;
        self.spawn_campfire(
            placement,
            floor_level,
            onlinerpg_shared::hunger::CAMPFIRE_DURATION_MS,
        )
        .await;
        self.send_system_message(player_id, "You light a campfire.")
            .await;
    }

    pub(super) async fn campfire_placement(
        &self,
        player_id: &PlayerId,
    ) -> Option<(crate::types::Position, i8)> {
        self.outdoor_placement(
            player_id,
            PLACEMENT_DISTANCE_M,
            "You can only build a campfire outdoors",
            "You can't light a fire in water",
        )
        .await
    }

    /// Where something placed by `player_id` would land: `distance_m` in front
    /// of them, or their own feet when something blocks the way. `None` once
    /// the refusal (indoors, in water) has been sent to them.
    pub(super) async fn outdoor_placement(
        &self,
        player_id: &PlayerId,
        distance_m: f32,
        indoors_refusal: &str,
        water_refusal: &str,
    ) -> Option<(crate::types::Position, i8)> {
        let (position, rotation, floor_level) = {
            let players = self.players.read().await;
            let p = players.get(player_id)?;
            (p.position, p.rotation, p.floor_level)
        };
        if floor_level != super::fishing::OVERWORLD_FLOOR {
            self.send_system_message(player_id, indoors_refusal).await;
            return None;
        }
        let forward = crate::types::Position {
            x: position.x + rotation.sin() * distance_m,
            y: position.y,
            z: position.z + rotation.cos() * distance_m,
        };
        let placement = {
            let cache = self.passability_read();
            let floor = super::passability::authoritative_floor(&cache, &position);
            if super::passability::wrapped_block_info(
                &cache, position.x, position.z, forward.x, forward.z, floor, position.y,
            )
            .is_some()
            {
                position
            } else {
                forward.wrapped_x()
            }
        };
        let wx = onlinerpg_shared::wrap_world_x(placement.x);
        let in_water = self
            .water_depth_at(wx, placement.z)
            .await
            .is_some_and(|depth| depth > onlinerpg_shared::fishing::MIN_FISHABLE_DEPTH_M);
        if in_water {
            self.send_system_message(player_id, water_refusal).await;
            return None;
        }
        Some((placement, floor_level))
    }

    /// Roll `dice` and heal an alive, wounded player, broadcasting the new HP.
    async fn roll_heal_and_broadcast(&self, player_id: &PlayerId, dice: &str) {
        let healed = {
            let mut players = self.players.write().await;
            players.get_mut(player_id).and_then(|player| {
                (player.health > 0 && player.health < player.max_health).then(|| {
                    let amount = crate::game::combat::roll_dice(dice);
                    player.health = (player.health + amount).min(player.max_health);
                    (
                        player.health,
                        player.max_health,
                        player.position,
                        player.floor_level,
                    )
                })
            })
        };
        if let Some((health, max_health, position, floor_level)) = healed {
            self.mark_party_vitals_dirty(player_id).await;
            self.send_direct_message_to_players_within_position(
                &position,
                floor_level,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::PlayerHealthUpdate {
                    player_id: *player_id,
                    health,
                    max_health,
                },
                None,
            )
            .await;
        }
    }

    /// If the player is defeated (or gone), message them and return true so
    /// the caller can bail. Shared guard for read-a-scroll style consumables.
    pub(super) async fn reject_if_defeated(&self, player_id: &PlayerId, message: &str) -> bool {
        let defeated = match self.players.read().await.get(player_id) {
            Some(player) => player.health == 0,
            None => return true,
        };
        if defeated {
            self.send_system_message(player_id, message).await;
        }
        defeated
    }

    /// Read a scroll of return: whisk the reader back to the town spawn
    /// (surface floor). Refuses while defeated so the dead can't escape death.
    async fn use_return_scroll(&self, player_id: &PlayerId, instance_id: u64) {
        if self
            .reject_if_defeated(player_id, "You can't read while defeated")
            .await
        {
            return;
        }

        self.consume_one_and_sync(player_id, instance_id).await;
        self.teleport_to_town(player_id).await;
    }

    /// Read a scroll of party summon: ask every other online party member to
    /// teleport to the reader's side. Refuses — keeping the scroll — while
    /// defeated, in combat, or with no one to call, like the enchant
    /// scroll's guard.
    async fn use_party_summon_scroll(&self, player_id: &PlayerId, instance_id: u64) {
        if self
            .reject_if_defeated(player_id, "You can't read while defeated")
            .await
        {
            return;
        }

        // A summons gathers a party in peace; escape under fire is the
        // return scroll's job. Reading waits out the same clock that gates
        // accepting, so a fight (or a future PvP blob) can't open with one.
        let in_combat = {
            let players = self.players.read().await;
            players.get(player_id).is_some_and(Self::in_combat)
        };
        if in_combat {
            self.send_system_message(player_id, "You can't read this while in combat")
                .await;
            return;
        }

        match self.try_cast_party_summon(player_id).await {
            SummonCast::Called => self.consume_one_and_sync(player_id, instance_id).await,
            SummonCast::NoMembers => {
                self.send_system_message(player_id, "Summon: no party members to call.")
                    .await
            }
            SummonCast::CallStillOut => {
                self.send_system_message(player_id, "Summon: your call is still out.")
                    .await
            }
        }
    }

    /// Read a scroll of enchant weapon: +1 to the wielded weapon, added to
    /// attack and damage rolls.
    async fn use_enchant_weapon_scroll(&self, player_id: &PlayerId, instance_id: u64) {
        self.read_enchant_scroll(
            player_id,
            instance_id,
            EnchantScroll {
                // Only a wielded weapon bites; an empty or non-weapon main
                // hand keeps the scroll unread.
                select: |inv, defs, _| {
                    inv.equipped
                        .get(&EquipSlot::MainHand)
                        .filter(|item| defs.get(&item.item_def_id).is_some_and(|d| d.is_weapon()))
                        .map(|_| EquipSlot::MainHand)
                },
                ladder: enchant_success_bp,
                no_target: "You have no weapon wielded to enchant",
                destroyed: |name| {
                    format!(
                        "The runes flare out of control — your {name} bursts into glittering dust!"
                    )
                },
                honed: |name, enchant| {
                    format!("The runes sink into your {name}, honing its edge. (+{enchant})")
                },
            },
        )
        .await;
    }

    /// Read a scroll of enchant armor: +1 to one random worn armor piece,
    /// added to the wearer's guard.
    async fn use_enchant_armor_scroll(&self, player_id: &PlayerId, instance_id: u64) {
        self.read_enchant_scroll(
            player_id,
            instance_id,
            EnchantScroll {
                select: |inv, defs, pick| {
                    let worn: Vec<EquipSlot> = inv
                        .equipped
                        .iter()
                        .filter(|(_, item)| {
                            defs.get(&item.item_def_id).is_some_and(|d| d.is_armor())
                        })
                        .map(|(slot, _)| *slot)
                        .collect();
                    (!worn.is_empty()).then(|| worn[pick as usize % worn.len()])
                },
                ladder: armor_enchant_success_bp,
                no_target: "You have no armor worn to enchant",
                destroyed: |name| {
                    format!("The runes flare out of control — your {name} crumbles to dust!")
                },
                honed: |name, enchant| {
                    format!("The runes sink into your {name}, hardening it. (+{enchant})")
                },
            },
        )
        .await;
    }

    /// The ceremony both enchant scrolls share: refuse while defeated or with
    /// nothing to target (keeping the scroll), else spend the scroll and either
    /// raise the piece by one or destroy it on the odds ladder's roll.
    async fn read_enchant_scroll(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        scroll: EnchantScroll,
    ) {
        if self
            .reject_if_defeated(player_id, "You can't read while defeated")
            .await
        {
            return;
        }
        // The oil is drawn by definition, not by instance, so it cannot be
        // reconciled against a trade table's instance-level reservations.
        if self
            .reject_if_trading(player_id, "read an enchant scroll")
            .await
        {
            return;
        }

        // Rolled before the lock is taken; `pick` chooses among the targets
        // the selector finds.
        let (roll_bp, pick) = {
            let mut rng = rand::thread_rng();
            (rng.gen_range(0..ENCHANT_BP_SCALE), rng.gen::<u64>())
        };

        let (snapshot, message, enchant_log, scroll_def) = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return,
            };

            let Some(slot) = (scroll.select)(inv, &self.item_defs, pick) else {
                drop(inventories);
                self.send_system_message(player_id, scroll.no_target).await;
                return;
            };

            // No oil, no reading: the scroll stays in the bag.
            if draw_from_bag(&mut inv.bag, WHETSTONE_OIL_ITEM_ID, 1).is_empty() {
                drop(inventories);
                self.send_system_message(
                    player_id,
                    "You need Whetstone Oil to prepare the gear before the runes bite.",
                )
                .await;
                return;
            }

            // Spent whether the enchant takes or the piece breaks.
            let scroll_def = consume_one(inv, instance_id);

            let item = inv.equipped.get_mut(&slot).expect("the selector found it");
            let name = self.item_name(&item.item_def_id);
            let (message, enchant_log) = if roll_bp >= (scroll.ladder)(item.enchant) {
                let log = format!(
                    "destroyed {} enchanting at +{}",
                    item.item_def_id, item.enchant
                );
                inv.equipped.remove(&slot);
                ((scroll.destroyed)(&name), log)
            } else {
                item.enchant += 1;
                (
                    (scroll.honed)(&name, item.enchant),
                    format!("enchanted {} to +{}", item.item_def_id, item.enchant),
                )
            };
            (inv.clone(), message, enchant_log, scroll_def)
        };

        let name = self.player_name_of(player_id).await;
        info!("{name} {enchant_log}");
        // The reading's reagents skip consume_one_and_sync, so journal-based
        // consumption metrics need their own lines here.
        if let Some((position, _, floor_level, _)) = self.player_pose(player_id).await {
            let place = crate::dungeon_defs::place_label(&position, floor_level);
            info!("{name} consumed {WHETSTONE_OIL_ITEM_ID} at {place}");
            if let Some(scroll_def) = scroll_def {
                info!("{name} consumed {scroll_def} at {place}");
            }
        }
        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        self.send_system_message(player_id, message).await;
    }

    /// Display name for an item def, falling back to the raw id.
    pub(super) fn item_name(&self, item_def_id: &str) -> String {
        self.item_defs
            .get(item_def_id)
            .map(|def| def.name.clone())
            .unwrap_or_else(|| item_def_id.to_string())
    }

    /// Remove one unit of `instance_id` from the player's bag (dropping the
    /// instance when the stack empties), persist, and push the fresh snapshot
    /// to the client.
    /// Whether the player carries at least one `item_def_id`, bag or worn.
    pub(super) async fn holds_item(&self, player_id: &PlayerId, item_def_id: &str) -> bool {
        self.inventories
            .read()
            .await
            .get(player_id)
            .is_some_and(|inv| inv.has_item(item_def_id))
    }

    pub(super) async fn consume_one_and_sync(&self, player_id: &PlayerId, instance_id: u64) {
        let (snapshot, item_def_id) = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return,
            };
            let def_id = consume_one(inv, instance_id);
            (inv.clone(), def_id)
        };

        if let Some(def_id) = item_def_id {
            if let Some((position, _, floor_level, name)) = self.player_pose(player_id).await {
                let place = crate::dungeon_defs::place_label(&position, floor_level);
                info!("{name} consumed {def_id} at {place}");
            }
        }

        self.mark_dirty(player_id).await;
        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
    }

    /// Insert a ground item into the world and announce it to nearby players.
    /// Visible and pickable the moment this runs; a caller that owes the drop
    /// an animation beat delays this call (`spawn_kill_loot_after_impact`).
    pub(super) async fn spawn_ground_item(&self, ground_item: GroundItem) {
        let position = ground_item.position;
        let floor_level = ground_item.floor_level;
        {
            let mut ground_items = self.ground_items.write().await;
            ground_items.insert(
                ground_item.instance_id,
                ServerGroundItem {
                    item: ground_item.clone(),
                    dropped_at_ms: Self::now_ms(),
                },
            );
        }
        self.send_direct_message_to_players_within_position(
            &position,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::GroundItemSpawned { item: ground_item },
            None,
        )
        .await;
    }

    /// Roll the global world-drop table for a loot event at `origin` and spawn
    /// any rare bonus items that hit as ground items scattered nearby. Shared
    /// by every loot source (monster kills, dungeon chests, broken props) so a
    /// rare drop can spill from anything that yields loot. Table entries are
    /// validated against `ItemDefs` at load time, so every rolled id is
    /// guaranteed to have a definition here. `bonus_item_ids` (a monster's own
    /// pre-rolled drops) ride along in the same scatter pass.
    ///
    /// `source_level` is the killed monster's effective level, which some
    /// entries pay out less on when it is low (`world_drop.csv`). Chests and
    /// props have no level and pass `None` — they roll at full chance, since
    /// each one only yields loot once per dungeon instance.
    pub(super) async fn spawn_world_drops(
        &self,
        origin: crate::types::Position,
        floor_level: i8,
        source_level: Option<u8>,
        bonus_item_ids: Vec<String>,
    ) {
        /// How far from the loot origin a world drop scatters.
        const WORLD_DROP_OFFSET_METERS: f32 = 1.5;

        let mut item_def_ids = bonus_item_ids;
        {
            let mut rng = rand::thread_rng();
            item_def_ids.extend(self.world_drop_defs.roll(&mut rng, source_level));
        }
        if !item_def_ids.is_empty() {
            info!(
                "Bonus drops {:?} at ({:.1},{:.1}) {}",
                item_def_ids,
                origin.x,
                origin.z,
                crate::dungeon_defs::place_label(&origin, floor_level)
            );
        }
        self.spawn_scattered_items(
            item_def_ids,
            origin,
            floor_level,
            WORLD_DROP_OFFSET_METERS,
            WORLD_DROP_OFFSET_METERS,
        )
        .await;
    }

    /// Spawn items as ground drops scattered around `origin`, each at a random
    /// angle and a radius in `min_r..=max_r`. Each drop is clamped onto
    /// walkable floor inside dungeons so it never lands in a wall; anyone may
    /// pick them up.
    pub(super) async fn spawn_scattered_items(
        &self,
        item_def_ids: Vec<String>,
        origin: crate::types::Position,
        floor_level: i8,
        min_r: f32,
        max_r: f32,
    ) {
        use std::f32::consts::TAU;

        for item_def_id in item_def_ids {
            let angle = rand::thread_rng().gen_range(0.0..TAU);
            let radius = rand::thread_rng().gen_range(min_r..=max_r);
            let preferred = super::combat::offset_position_at_angle(origin, angle, radius);
            let position = self
                .loot_drop_position(origin, floor_level, preferred)
                .await;

            let instance_id = self.next_instance_id().await;
            self.spawn_ground_item(GroundItem {
                instance_id,
                item_def_id,
                position,
                floor_level,
                quantity: 1,
                enchant: 0,
                dropped_by: None,
                cape_color: None,
                cape_texture: None,
            })
            .await;
        }
    }

    pub async fn drop_item(&self, player_id: &PlayerId, instance_id: u64) {
        if self
            .reject_if_trade_reserved(player_id, instance_id, "drop")
            .await
        {
            return;
        }
        let (player_position, rotation, floor_level) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.position, p.rotation, p.floor_level),
                None => return,
            }
        };
        // Scatter, or a run of drops piles onto one pixel under the player.
        let preferred = drop_landing_position(player_position, rotation);
        let position = self
            .loot_drop_position(player_position, floor_level, preferred)
            .await;
        // For the unit that splits off a stack — the stack keeps its own id.
        // Reserved outside the lock like award_item; unused otherwise.
        let split_instance_id = self.next_instance_id().await;
        let npc_def = self.official_npc_def(player_id).await;

        let (snapshot, dropped, dropped_from_off_hand) = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return,
            };

            // Dropped loadout gear would be lootable and re-seeded on the
            // NPC's next join — an item faucet, like selling it.
            if npc_def.is_some_and(|def| {
                inv.items()
                    .any(|i| i.instance_id == instance_id && def.in_loadout(&i.item_def_id))
            }) {
                drop(inventories);
                self.send_system_message(player_id, "You never drop your issued gear")
                    .await;
                return;
            }

            let (dropped, dropped_from_off_hand) =
                if let Some(idx) = inv.bag.iter().position(|i| i.instance_id == instance_id) {
                    if inv.bag[idx].quantity > 1 {
                        inv.bag[idx].quantity -= 1;
                        let unit = ItemInstance {
                            instance_id: split_instance_id,
                            item_def_id: inv.bag[idx].item_def_id.clone(),
                            quantity: 1,
                            enchant: inv.bag[idx].enchant,
                            cape_color: inv.bag[idx].cape_color.clone(),
                            cape_texture: inv.bag[idx].cape_texture.clone(),
                        };
                        (unit, false)
                    } else {
                        (inv.bag.remove(idx), false)
                    }
                } else if let Some(slot) = inv
                    .equipped
                    .iter()
                    .find(|(_, item)| item.instance_id == instance_id)
                    .map(|(slot, _)| *slot)
                {
                    (
                        inv.equipped.remove(&slot).expect("checked above"),
                        slot == EquipSlot::OffHand,
                    )
                } else {
                    drop(inventories);
                    self.send_system_message(player_id, "Item not found").await;
                    return;
                };

            (inv.clone(), dropped, dropped_from_off_hand)
        };

        let ground_item = GroundItem {
            instance_id: dropped.instance_id,
            item_def_id: dropped.item_def_id,
            position,
            floor_level,
            quantity: 1,
            enchant: dropped.enchant,
            cape_color: dropped.cape_color,
            cape_texture: dropped.cape_texture,
            dropped_by: Some(*player_id),
        };

        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        if dropped_from_off_hand {
            self.set_player_torch(player_id, false).await;
        }
        self.spawn_ground_item(ground_item).await;
        // Dropping the equipped rod is as much "putting it away" as
        // unequipping it — same mid-session abort.
        self.abort_fishing_if_rod_lost(player_id).await;
        self.abort_instrument_if_lost(player_id).await;
    }

    /// Drop multiple bag stacks (partial quantities allowed) in one
    /// all-or-nothing transaction: every line is validated against the bag
    /// before anything is removed. Bag-only — unlike `drop_item`, there is no
    /// equipped-slot fallback, since batch selection only ever offers bag
    /// items.
    pub async fn drop_items(&self, player_id: &PlayerId, mut items: Vec<BagLineItem>) {
        items.retain(|i| i.qty > 0);
        if items.is_empty() {
            return;
        }
        if self.reject_if_trading(player_id, "drop").await {
            return;
        }
        let Some(quantities) =
            super::checked_batch_quantities(items.iter().map(|item| (item.instance_id, item.qty)))
        else {
            self.send_system_message(player_id, "Invalid batch quantity")
                .await;
            return;
        };
        let (player_position, rotation, floor_level) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.position, p.rotation, p.floor_level),
                None => return,
            }
        };

        struct Plan {
            instance_id: u64,
            qty: u32,
            item_def_id: String,
            enchant: i32,
            cape_color: Option<String>,
            cape_texture: Option<String>,
        }

        let npc_def = self.official_npc_def(player_id).await;
        let mut plans: Vec<Plan> = Vec::with_capacity(items.len());

        let snapshot = {
            let mut inventories = self.inventories.write().await;
            let Some(inv) = inventories.get_mut(player_id) else {
                return;
            };

            for req in &items {
                let Some(item) = inv.bag.iter().find(|i| i.instance_id == req.instance_id) else {
                    drop(inventories);
                    self.send_system_message(player_id, "Item not found").await;
                    return;
                };
                if quantities[&req.instance_id] > item.quantity {
                    drop(inventories);
                    self.send_system_message(player_id, "Not enough of that item")
                        .await;
                    return;
                }
                if npc_def.is_some_and(|def| def.in_loadout(&item.item_def_id)) {
                    drop(inventories);
                    self.send_system_message(player_id, "You never drop your issued gear")
                        .await;
                    return;
                }
                plans.push(Plan {
                    instance_id: req.instance_id,
                    qty: req.qty,
                    item_def_id: item.item_def_id.clone(),
                    enchant: item.enchant,
                    cape_color: item.cape_color.clone(),
                    cape_texture: item.cape_texture.clone(),
                });
            }
            // Every line is now guaranteed to apply cleanly — mutate.
            for plan in &plans {
                let idx = inv
                    .bag
                    .iter()
                    .position(|i| i.instance_id == plan.instance_id)
                    .expect("checked above");
                if inv.bag[idx].quantity > plan.qty {
                    inv.bag[idx].quantity -= plan.qty;
                } else {
                    inv.bag.remove(idx);
                }
            }
            inv.clone()
        };

        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        self.abort_instrument_if_lost(player_id).await;

        // A stackable line lands as a single N-unit pile, a non-stackable one
        // scatters unit by unit since those units are distinct objects. One
        // (piles, per-pile) shape per plan keeps the id reservation and the
        // spawn loop counting the same way.
        let shapes: Vec<(u32, u32)> = plans
            .iter()
            .map(|plan| {
                if self.item_defs.stackable(&plan.item_def_id) {
                    (1, plan.qty)
                } else {
                    (plan.qty, 1)
                }
            })
            .collect();
        let total: u64 = shapes.iter().map(|(piles, _)| *piles as u64).sum();
        let mut next_ground_id = self.reserve_instance_ids(total).await;

        for (plan, &(piles, quantity)) in plans.iter().zip(&shapes) {
            for _ in 0..piles {
                let preferred = drop_landing_position(player_position, rotation);
                let position = self
                    .loot_drop_position(player_position, floor_level, preferred)
                    .await;
                self.spawn_ground_item(GroundItem {
                    instance_id: next_ground_id,
                    item_def_id: plan.item_def_id.clone(),
                    position,
                    floor_level,
                    quantity,
                    enchant: plan.enchant,
                    cape_color: plan.cape_color.clone(),
                    cape_texture: plan.cape_texture.clone(),
                    dropped_by: Some(*player_id),
                })
                .await;
                next_ground_id += 1;
            }
        }
    }

    pub async fn debug_drop_item(&self, player_id: &PlayerId, item_def_id: &str) {
        if self.item_defs.get(item_def_id).is_none() {
            self.send_system_message(player_id, "Unknown item").await;
            return;
        }

        let (player_position, rotation, floor_level) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.position, p.rotation, p.floor_level),
                None => return,
            }
        };

        let position = drop_landing_position(player_position, rotation);

        let instance_id = self.next_instance_id().await;
        self.spawn_ground_item(GroundItem {
            instance_id,
            item_def_id: item_def_id.to_string(),
            position,
            floor_level,
            quantity: 1,
            enchant: 0,
            dropped_by: Some(*player_id),
            cape_color: None,
            cape_texture: None,
        })
        .await;
    }

    pub async fn pickup_item(&self, player_id: &PlayerId, instance_id: u64) {
        let (player_pos, player_floor) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.position, p.floor_level),
                None => return,
            }
        };

        let ground_item = {
            let ground_items = self.ground_items.read().await;
            match ground_items.get(&instance_id) {
                Some(sgi) => sgi.item.clone(),
                None => {
                    self.send_system_message(player_id, "Item no longer exists")
                        .await;
                    return;
                }
            }
        };

        let dx = onlinerpg_shared::shortest_world_delta_x(ground_item.position.x, player_pos.x);
        let dz = player_pos.z - ground_item.position.z;
        if dx * dx + dz * dz > MAX_PICKUP_DISTANCE * MAX_PICKUP_DISTANCE {
            self.send_system_message(player_id, "Too far away").await;
            return;
        }

        // Exact floor match. Negative floors are dungeon depths now, so
        // the old "-1 matches any floor" wildcard is gone (outdoors and
        // house ground floors are both 0).
        if player_floor != ground_item.floor_level {
            self.send_system_message(player_id, "Item is on a different floor")
                .await;
            return;
        }

        // The dungeon coin pile is currency, not a bag item: picking it up
        // credits a few copper to the wallet instead of taking inventory space.
        if ground_item.item_def_id == super::COIN_PILE_ITEM_ID {
            self.pickup_coin_pile(player_id, instance_id, &ground_item, player_floor)
                .await;
            return;
        }

        let stackable = self.item_defs.stackable(&ground_item.item_def_id);
        let max_weight = self.max_carry_weight(player_id).await;
        let armor_mult = self.armor_weight_mult(player_id).await;
        let item_weight = self
            .item_defs
            .weight_with(&ground_item.item_def_id, armor_mult);
        // For the bag entry — the pile that stays behind keeps its own id.
        // Reserved outside the locks like `drop_item`'s split id; unused when
        // the insert merges into an existing bag stack.
        let bag_instance_id = self.next_instance_id().await;

        // Acquire write lock for both weight check and mutation atomically
        let item_position = ground_item.position;
        let (take, remaining, snapshot) = {
            let mut ground_items = self.ground_items.write().await;
            let Some(entry) = ground_items.get_mut(&instance_id) else {
                self.send_system_message(player_id, "Item no longer exists")
                    .await;
                return;
            };
            // Re-read under the lock: another picker may have thinned the pile
            // since the distance check above read it. A non-stackable ground
            // item is always a single object, and taking more would outrun the
            // one bag id reserved above.
            let available = if stackable { entry.item.quantity } else { 1 };

            let mut inventories = self.inventories.write().await;
            let Some(inv) = inventories.get_mut(player_id) else {
                return;
            };
            // Carry what fits and leave the rest, so a heavy pile is never
            // stranded on the ground with no way to take any of it.
            let headroom = max_weight - self.calc_total_weight(inv, armor_mult);
            let take = if item_weight <= 0.0 {
                available
            } else {
                // The epsilon keeps an exact fit from flooring one unit short
                // of what f32 rounding owes it (0.3-weight defs and friends).
                available.min(((headroom / item_weight) + 1e-3).floor().max(0.0) as u32)
            };
            if take == 0 {
                drop(inventories);
                drop(ground_items);
                self.send_system_message(player_id, "Too heavy to carry")
                    .await;
                return;
            }

            if take < available {
                entry.item.quantity = available - take;
            } else {
                ground_items.remove(&instance_id);
            }
            // Ids are never reused, so the pre-lock `ground_item` clone still
            // matches `entry` — no need to copy out of it under the locks.
            stack_into_bag(
                &mut inv.bag,
                BagInsert {
                    stackable,
                    item_def_id: &ground_item.item_def_id,
                    enchant: ground_item.enchant,
                    cape_color: ground_item.cape_color.clone(),
                    cape_texture: ground_item.cape_texture.clone(),
                    first_instance_id: bag_instance_id,
                    quantity: take,
                },
            );
            (take, available - take, inv.clone())
        };

        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        let update = if remaining == 0 {
            ServerMessage::GroundItemRemoved {
                instance_id,
                picked_up_by: Some(*player_id),
            }
        } else {
            ServerMessage::GroundItemQuantityChanged {
                instance_id,
                quantity: remaining,
                picked_up_by: Some(*player_id),
            }
        };
        self.send_direct_message_to_players_within_position(
            &item_position,
            player_floor,
            super::EVENT_DELIVERY_RADIUS,
            update,
            None,
        )
        .await;
        if remaining > 0 {
            self.send_system_message(
                player_id,
                &format!("Too heavy to carry it all — took {take}, left {remaining}."),
            )
            .await;
        }
    }

    /// Show the pickup crouch on nearby clients. Driven by `PickupStarted` at
    /// the clip's first frame, so remotes play it from the top rather than
    /// joining at the grab moment and finishing a third of a clip late.
    ///
    /// Transient: it bypasses the player's stored `object_type`, so no
    /// `StopInteraction` follows and a late joiner never sees a held pickup
    /// pose — remotes end the clip on their own. Not gated on the pickup
    /// succeeding: the player performed the motion either way, and the
    /// animation carries no item.
    pub async fn broadcast_pickup_animation(&self, player_id: &PlayerId) {
        let (position, floor_level) = {
            let players = self.players.read().await;
            match players.get(player_id) {
                Some(p) => (p.position, p.floor_level),
                None => return,
            }
        };
        self.send_direct_message_to_players_within_position(
            &position,
            floor_level,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::PlayerInteractionChanged {
                player_id: *player_id,
                object_type: Some("pickup".to_string()),
                object_id: None,
            },
            Some(player_id),
        )
        .await;
    }

    /// Credit loose copper to a player's wallet and tell them (`GoldUpdate` +
    /// `GoldGained`). Shared by coin-pile pickups and fished-up coin catches.
    pub(super) async fn award_copper(&self, player_id: &PlayerId, copper: i64) {
        let new_gold = {
            let mut gold_map = self.player_gold.write().await;
            let wallet = gold_map.entry(*player_id).or_insert(0);
            *wallet += copper;
            *wallet
        };
        self.mark_dirty(player_id).await;
        self.send_direct_message(player_id, ServerMessage::GoldUpdate { gold: new_gold })
            .await;
        self.send_direct_message(player_id, ServerMessage::GoldGained { amount: copper })
            .await;
    }

    /// `award_copper`'s debit twin: take `copper` if the wallet covers it and
    /// push the new balance. False (and nothing spent) when it doesn't.
    pub(super) async fn spend_copper(&self, player_id: &PlayerId, copper: i64) -> bool {
        let remaining = {
            let mut gold_map = self.player_gold.write().await;
            let wallet = gold_map.entry(*player_id).or_insert(0);
            if *wallet < copper {
                return false;
            }
            *wallet -= copper;
            *wallet
        };
        self.mark_dirty(player_id).await;
        self.send_direct_message(player_id, ServerMessage::GoldUpdate { gold: remaining })
            .await;
        true
    }

    /// Pick up a dungeon coin pile: claim it (first picker wins), credit a
    /// random 1–10 copper to the wallet, then broadcast its removal to nearby
    /// players. Skips the bag/weight path entirely — it's currency, not loot.
    async fn pickup_coin_pile(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        ground_item: &GroundItem,
        player_floor: i8,
    ) {
        // Claim the pile under the ground-items lock so two players racing for
        // the same pile can't both be paid.
        {
            let mut ground_items = self.ground_items.write().await;
            if ground_items.remove(&instance_id).is_none() {
                self.send_system_message(player_id, "Item no longer exists")
                    .await;
                return;
            }
        }

        let copper: i64 = rand::thread_rng().gen_range(1..=10);
        self.award_copper(player_id, copper).await;
        self.send_system_message(player_id, format!("You picked up {copper} copper."))
            .await;
        info!(
            "Player {} picked up a coin pile: +{} copper",
            self.player_name_of(player_id).await,
            copper
        );

        self.send_direct_message_to_players_within_position(
            &ground_item.position,
            player_floor,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::GroundItemRemoved {
                instance_id,
                picked_up_by: Some(*player_id),
            },
            None,
        )
        .await;
    }

    pub async fn tick_ground_item_despawn(&self) {
        let now = Self::now_ms();
        let mut to_remove = Vec::new();

        {
            let ground_items = self.ground_items.read().await;
            for (id, sgi) in ground_items.iter() {
                if now.saturating_sub(sgi.dropped_at_ms) > GROUND_ITEM_LIFETIME_MS {
                    to_remove.push(*id);
                }
            }
        }

        if to_remove.is_empty() {
            return;
        }

        let removed_items = {
            let mut ground_items = self.ground_items.write().await;
            to_remove
                .iter()
                .filter_map(|id| {
                    ground_items
                        .remove(id)
                        .map(|sgi| (*id, sgi.item.position, sgi.item.floor_level))
                })
                .collect::<Vec<_>>()
        };

        info!("Despawned {} ground item(s)", removed_items.len());
        for (id, position, floor_level) in removed_items {
            self.send_direct_message_to_players_within_position(
                &position,
                floor_level,
                super::EVENT_DELIVERY_RADIUS,
                ServerMessage::GroundItemRemoved {
                    instance_id: id,
                    picked_up_by: None,
                },
                None,
            )
            .await;
        }
    }

    pub async fn collect_dirty_inventory_states(
        &self,
    ) -> (Vec<PlayerId>, Vec<(i64, Vec<ItemRow>)>) {
        let dirty_ids: Vec<PlayerId> = {
            let mut dirty = self.dirty_inventories.write().await;
            dirty.drain().collect()
        };

        if dirty_ids.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let inventories = self.inventories.read().await;
        let player_chars = self.player_characters.read().await;

        let mut result = Vec::with_capacity(dirty_ids.len());
        for pid in &dirty_ids {
            if let (Some(inv), Some((char_id, _, _))) =
                (inventories.get(pid), player_chars.get(pid))
            {
                result.push((*char_id, serialize_inventory(inv)));
            }
        }

        (dirty_ids, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enchant_success_ladder_halves_past_seven_with_one_percent_floor() {
        // Guaranteed range.
        assert_eq!(enchant_success_bp(0), 10_000);
        assert_eq!(enchant_success_bp(4), 10_000);
        // Classic gamble steps.
        assert_eq!(enchant_success_bp(5), 7_500);
        assert_eq!(enchant_success_bp(6), 5_000);
        assert_eq!(enchant_success_bp(7), 2_500);
        // Halving ladder from +8.
        assert_eq!(enchant_success_bp(8), 1_250);
        assert_eq!(enchant_success_bp(9), 625);
        assert_eq!(enchant_success_bp(10), 312);
        assert_eq!(enchant_success_bp(11), 156);
        // 1% floor: 78bp would be below it, and it never drops further.
        assert_eq!(enchant_success_bp(12), 100);
        assert_eq!(enchant_success_bp(50), 100);
        assert_eq!(enchant_success_bp(i32::MAX), 100);
    }

    #[test]
    fn armor_enchant_ladder_is_the_weapon_one_shifted_two_levels_down() {
        assert_eq!(armor_enchant_success_bp(2), 10_000);
        assert_eq!(armor_enchant_success_bp(3), 7_500);
        assert_eq!(armor_enchant_success_bp(5), 2_500);
        assert_eq!(armor_enchant_success_bp(10), 100);
        assert_eq!(armor_enchant_success_bp(i32::MAX), 100);
    }
}

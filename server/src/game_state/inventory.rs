use std::collections::HashMap;

use crate::auth::{AuthService, ItemRow};
use crate::item_defs::UseEffect;
use crate::types::{PlayerId, ServerMessage};
use crate::world_config::world_config;
use onlinerpg_shared::inventory::{EquipSlot, GroundItem, ItemInstance, PlayerInventory};
use onlinerpg_shared::messages::BagLineItem;
use rand::Rng;
use tracing::{info, warn};

use super::party::SummonCast;
use super::ServerGroundItem;

/// Ground items despawn after 5 minutes.
const GROUND_ITEM_LIFETIME_MS: u64 = 5 * 60 * 1000;

const MAX_PICKUP_DISTANCE: f32 = 2.5;

const CAMPFIRE_PLACEMENT_DISTANCE_M: f32 = 1.0;

/// Enchant odds are expressed in basis points (1/100 of a percent) out of
/// this scale; the handler's roll must use the same bound.
const ENCHANT_BP_SCALE: u32 = 10_000;

/// Success chance, in basis points, of enchanting a weapon currently at
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

/// One unit-insert request: `quantity` units of one def at one enchant level,
/// backed by ids starting at `first_instance_id`.
pub(super) struct BagInsert<'a> {
    pub stackable: bool,
    pub item_def_id: &'a str,
    pub enchant: i32,
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
        }
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
        first_instance_id,
        quantity,
    } = insert;
    if quantity == 0 {
        return 0;
    }

    if stackable {
        if let Some(stack) = bag
            .iter_mut()
            .find(|item| item.item_def_id == item_def_id && item.enchant == enchant)
        {
            stack.quantity += quantity;
            return 0;
        }
        bag.push(ItemInstance {
            instance_id: first_instance_id,
            item_def_id: item_def_id.to_string(),
            quantity,
            enchant,
        });
        return 1;
    }

    for offset in 0..quantity as u64 {
        bag.push(ItemInstance {
            instance_id: first_instance_id + offset,
            item_def_id: item_def_id.to_string(),
            quantity: 1,
            enchant,
        });
    }
    quantity as u64
}

/// Remove one unit of `instance_id` from the bag, dropping the instance when
/// the stack empties.
fn consume_one(inv: &mut PlayerInventory, instance_id: u64) {
    if let Some(idx) = inv.bag.iter().position(|i| i.instance_id == instance_id) {
        if inv.bag[idx].quantity > 1 {
            inv.bag[idx].quantity -= 1;
        } else {
            inv.bag.remove(idx);
        }
    }
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
        })
        .collect();
    for (slot, item) in &inv.equipped {
        rows.push(ItemRow {
            item_def_id: item.item_def_id.clone(),
            quantity: 1,
            equip_slot: Some(slot.as_str().to_string()),
            enchant: item.enchant,
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
    /// purchases — it never blocks movement.
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

    pub(super) fn calc_total_weight(&self, inventory: &PlayerInventory) -> f32 {
        let bag_weight: f32 = inventory
            .bag
            .iter()
            .map(|item| self.item_defs.weight(&item.item_def_id) * item.quantity as f32)
            .sum();
        let equip_weight: f32 = inventory
            .equipped
            .values()
            .map(|item| self.item_defs.weight(&item.item_def_id))
            .sum();
        bag_weight + equip_weight
    }

    /// Send the current inventory state directly to a player, then their
    /// refreshed guard. Every equipped-gear mutation routes through here, so
    /// pushing the guard (and the main-hand broadcast, which no-ops when
    /// unchanged) from this one spot keeps everything in sync without each
    /// mutation site having to remember to send it.
    async fn send_inventory_snapshot(&self, player_id: &PlayerId, inventory: PlayerInventory) {
        self.set_player_main_hand(player_id, inventory.main_hand_def_id())
            .await;
        self.send_direct_message(player_id, ServerMessage::InventoryUpdated { inventory })
            .await;
        self.send_guard_update(player_id).await;
    }

    /// Recompute and push the player's effective guard to their client, so the
    /// displayed value stays equal to the one combat resolves against.
    async fn send_guard_update(&self, player_id: &PlayerId) {
        let guard = self.effective_guard(player_id).await;
        self.send_direct_message(player_id, ServerMessage::GuardUpdated { guard })
            .await;
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
                                first_instance_id: next_id,
                                quantity: row.quantity,
                            },
                        );
                    }
                }
            }
        }

        let mut inventories = self.inventories.write().await;
        inventories.insert(*player_id, inventory);
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
        let Some((def_weight, stackable)) = self
            .item_defs
            .get(item_def_id)
            .map(|d| (d.weight, d.stackable))
        else {
            warn!("award_item: unknown item_def_id {:?}", item_def_id);
            return;
        };
        let max_weight = self.max_carry_weight(player_id).await;
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
            if self.calc_total_weight(inv) + def_weight > max_weight {
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
                    enchant: 0,
                })
                .await;
            }
        }
    }

    pub async fn equip_item(&self, player_id: &PlayerId, instance_id: u64) {
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

            let target_slot = if inv.equipped.contains_key(&equip_slot) {
                equip_slot
                    .alternate()
                    .filter(|alt| !inv.equipped.contains_key(alt))
                    .unwrap_or(equip_slot)
            } else {
                equip_slot
            };

            let item = inv.bag.remove(bag_idx);
            if let Some(old_item) = inv.equipped.insert(target_slot, item) {
                inv.bag.push(old_item);
            }
            let torch_on = (target_slot == EquipSlot::OffHand).then(|| inv.is_torch_lit());
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
            UseEffect::Eat {
                nutrition,
                heal_dice,
                raw_fish,
            } => {
                self.use_eat_item(player_id, instance_id, nutrition, heal_dice, raw_fish, None)
                    .await
            }
            UseEffect::PlaceCampfire => self.use_campfire_kit(player_id, instance_id).await,
            UseEffect::TeleportTown => self.use_return_scroll(player_id, instance_id).await,
            UseEffect::EnchantWeapon => {
                self.use_enchant_weapon_scroll(player_id, instance_id).await
            }
            UseEffect::SummonParty => self.use_party_summon_scroll(player_id, instance_id).await,
            UseEffect::OpenCoinPouch(dice) => {
                self.use_coin_pouch(player_id, instance_id, &dice).await
            }
        }
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

    /// Eat food or fish (doc/HUNGER.md): raw fish near a campfire grills
    /// instead. Unlike potions, full HP just skips the heal component;
    /// untracked (NPC) eaters get heal + decrement only. `force_poison`
    /// pins the raw-fish roll for tests.
    pub(super) async fn use_eat_item(
        &self,
        player_id: &PlayerId,
        instance_id: u64,
        nutrition: u32,
        heal_dice: Option<String>,
        raw_fish: bool,
        force_poison: Option<bool>,
    ) {
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

        let outcome = self
            .apply_eat(player_id, nutrition, raw_fish, force_poison)
            .await;
        if matches!(outcome, super::hunger::EatOutcome::TooStuffed) {
            self.send_system_message(player_id, "You are too stuffed to eat another bite")
                .await;
            return;
        }

        if let Some(dice) = heal_dice {
            self.roll_heal_and_broadcast(player_id, &dice).await;
        }

        self.consume_one_and_sync(player_id, instance_id).await;
        let name = self.item_name(&def_id);
        self.send_system_message(player_id, format!("You eat the {name}."))
            .await;
        if let super::hunger::EatOutcome::Fed(msg) = outcome {
            self.mark_dirty(player_id).await;
            self.send_direct_message(player_id, msg).await;
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
        let (position, rotation, floor_level) = {
            let players = self.players.read().await;
            let Some(p) = players.get(player_id) else {
                return;
            };
            (p.position, p.rotation, p.floor_level)
        };
        if floor_level != super::fishing::OVERWORLD_FLOOR {
            self.send_system_message(player_id, "You can only build a campfire outdoors")
                .await;
            return;
        }
        let forward = crate::types::Position {
            x: position.x + rotation.sin() * CAMPFIRE_PLACEMENT_DISTANCE_M,
            y: position.y,
            z: position.z + rotation.cos() * CAMPFIRE_PLACEMENT_DISTANCE_M,
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
            self.send_system_message(player_id, "You can't light a fire in water")
                .await;
            return;
        }

        self.consume_one_and_sync(player_id, instance_id).await;
        self.spawn_campfire(placement, floor_level).await;
        self.send_system_message(player_id, "You light a campfire.")
            .await;
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
    async fn reject_if_defeated(&self, player_id: &PlayerId, message: &str) -> bool {
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

        let spawn = &world_config().spawn_position;
        self.teleport_player(player_id, spawn.position(), spawn.rotation, 0)
            .await;
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

    /// Read a scroll of enchant weapon: +1 to the wielded weapon's
    /// enchantment, which is added to attack and damage rolls. NetHack-style
    /// over-enchanting gamble: past +4 each further reading risks destroying
    /// the weapon (see `enchant_success_bp` for the odds ladder).
    /// Refuses — keeping the scroll — while defeated or with nothing wielded.
    async fn use_enchant_weapon_scroll(&self, player_id: &PlayerId, instance_id: u64) {
        if self
            .reject_if_defeated(player_id, "You can't read while defeated")
            .await
        {
            return;
        }

        // Roll before taking the lock; a no-op while the weapon is still in
        // the guaranteed range.
        let roll_bp = rand::thread_rng().gen_range(0..ENCHANT_BP_SCALE);

        let (snapshot, message) = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return,
            };

            // The scroll only bites on a wielded weapon; an empty (or
            // non-weapon) main hand keeps it unread.
            let wielding_weapon = inv.equipped.get(&EquipSlot::MainHand).is_some_and(|item| {
                self.item_defs
                    .get(&item.item_def_id)
                    .is_some_and(|def| def.is_weapon())
            });
            if !wielding_weapon {
                drop(inventories);
                self.send_system_message(player_id, "You have no weapon wielded to enchant")
                    .await;
                return;
            }

            // The scroll is spent whether the enchant takes or the weapon breaks.
            consume_one(inv, instance_id);

            let weapon = inv
                .equipped
                .get_mut(&EquipSlot::MainHand)
                .expect("checked above");
            let name = self.item_name(&weapon.item_def_id);
            let message = if roll_bp >= enchant_success_bp(weapon.enchant) {
                inv.equipped.remove(&EquipSlot::MainHand);
                format!("The runes flare out of control — your {name} bursts into glittering dust!")
            } else {
                weapon.enchant += 1;
                format!(
                    "The runes sink into your {name}, honing its edge. (+{})",
                    weapon.enchant
                )
            };
            (inv.clone(), message)
        };

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
    pub(super) async fn consume_one_and_sync(&self, player_id: &PlayerId, instance_id: u64) {
        let snapshot = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return,
            };
            consume_one(inv, instance_id);
            inv.clone()
        };

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
    /// rare drop can spill from anything that yields loot. Each drop is
    /// clamped onto walkable floor inside dungeons so it never lands in a wall.
    /// Table entries are validated against `ItemDefs` at load time, so every
    /// rolled id is guaranteed to have a definition here.
    pub(super) async fn spawn_world_drops(&self, origin: crate::types::Position, floor_level: i8) {
        use std::f32::consts::TAU;

        /// How far from the loot origin a world drop scatters.
        const WORLD_DROP_OFFSET_METERS: f32 = 1.5;

        let item_def_ids = {
            let mut rng = rand::thread_rng();
            self.world_drop_defs.roll(&mut rng)
        };

        for item_def_id in item_def_ids {
            let angle = rand::thread_rng().gen_range(0.0..TAU);
            let preferred =
                super::combat::offset_position_at_angle(origin, angle, WORLD_DROP_OFFSET_METERS);
            let position = self
                .loot_drop_position(origin, floor_level, preferred)
                .await;

            let instance_id = self.next_instance_id().await;
            self.spawn_ground_item(GroundItem {
                instance_id,
                item_def_id,
                position,
                floor_level,
                enchant: 0,
            })
            .await;
        }
    }

    pub async fn drop_item(&self, player_id: &PlayerId, instance_id: u64) {
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

        let (snapshot, dropped, dropped_from_off_hand) = {
            let mut inventories = self.inventories.write().await;
            let inv = match inventories.get_mut(player_id) {
                Some(inv) => inv,
                None => return,
            };

            let (dropped, dropped_from_off_hand) =
                if let Some(idx) = inv.bag.iter().position(|i| i.instance_id == instance_id) {
                    if inv.bag[idx].quantity > 1 {
                        inv.bag[idx].quantity -= 1;
                        let unit = ItemInstance {
                            instance_id: split_instance_id,
                            item_def_id: inv.bag[idx].item_def_id.clone(),
                            quantity: 1,
                            enchant: inv.bag[idx].enchant,
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
            enchant: dropped.enchant,
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
    }

    /// Drop multiple bag stacks (partial quantities allowed) in one
    /// all-or-nothing transaction: every line is validated against the bag
    /// before anything is removed. Bag-only — unlike `drop_item`, there is no
    /// equipped-slot fallback, since batch selection only ever offers bag
    /// items. `GroundItem` has no quantity field, so each unit lands as its
    /// own scattered ground item rather than one N-quantity entity.
    pub async fn drop_items(&self, player_id: &PlayerId, items: Vec<BagLineItem>) {
        if items.is_empty() {
            return;
        }
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
        }

        let mut reserved: HashMap<u64, u32> = HashMap::new();
        let mut plans: Vec<Plan> = Vec::with_capacity(items.len());

        let snapshot = {
            let mut inventories = self.inventories.write().await;
            let Some(inv) = inventories.get_mut(player_id) else {
                return;
            };

            for req in &items {
                if req.qty == 0 {
                    continue;
                }
                let Some(item) = inv.bag.iter().find(|i| i.instance_id == req.instance_id) else {
                    drop(inventories);
                    self.send_system_message(player_id, "Item not found").await;
                    return;
                };
                let already = *reserved.get(&req.instance_id).unwrap_or(&0);
                if already + req.qty > item.quantity {
                    drop(inventories);
                    self.send_system_message(player_id, "Not enough of that item")
                        .await;
                    return;
                }
                reserved.insert(req.instance_id, already + req.qty);
                plans.push(Plan {
                    instance_id: req.instance_id,
                    qty: req.qty,
                    item_def_id: item.item_def_id.clone(),
                    enchant: item.enchant,
                });
            }
            if plans.is_empty() {
                drop(inventories);
                return;
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

        let total_units: u32 = plans.iter().map(|p| p.qty).sum();
        let mut next_ground_id = self.reserve_instance_ids(total_units as u64).await;
        for plan in &plans {
            for _ in 0..plan.qty {
                let preferred = drop_landing_position(player_position, rotation);
                let position = self
                    .loot_drop_position(player_position, floor_level, preferred)
                    .await;
                self.spawn_ground_item(GroundItem {
                    instance_id: next_ground_id,
                    item_def_id: plan.item_def_id.clone(),
                    position,
                    floor_level,
                    enchant: plan.enchant,
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
            enchant: 0,
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

        let item_weight = self.item_defs.weight(&ground_item.item_def_id);
        let stackable = self.item_defs.stackable(&ground_item.item_def_id);
        let max_weight = self.max_carry_weight(player_id).await;

        // Acquire write lock for both weight check and mutation atomically
        let item_position = ground_item.position;
        let snapshot = {
            let mut ground_items = self.ground_items.write().await;
            if ground_items.remove(&instance_id).is_none() {
                self.send_system_message(player_id, "Item no longer exists")
                    .await;
                return;
            }

            let mut inventories = self.inventories.write().await;
            if let Some(inv) = inventories.get_mut(player_id) {
                let current_weight = self.calc_total_weight(inv);
                if current_weight + item_weight > max_weight {
                    // Put it back on the ground
                    ground_items.insert(
                        instance_id,
                        ServerGroundItem {
                            item: ground_item,
                            dropped_at_ms: Self::now_ms(),
                        },
                    );
                    drop(inventories);
                    drop(ground_items);
                    self.send_system_message(player_id, "Too heavy to carry")
                        .await;
                    return;
                }
                stack_into_bag(
                    &mut inv.bag,
                    BagInsert::one(
                        stackable,
                        &ground_item.item_def_id,
                        ground_item.enchant,
                        instance_id,
                    ),
                );
                inv.clone()
            } else {
                return;
            }
        };

        self.mark_inventory_dirty(player_id).await;
        self.send_inventory_snapshot(player_id, snapshot).await;
        self.send_direct_message_to_players_within_position(
            &item_position,
            player_floor,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::GroundItemRemoved { instance_id },
            None,
        )
        .await;
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
        info!(
            "Player {} picked up a coin pile: +{} copper",
            self.player_name_of(player_id).await,
            copper
        );

        self.send_direct_message_to_players_within_position(
            &ground_item.position,
            player_floor,
            super::EVENT_DELIVERY_RADIUS,
            ServerMessage::GroundItemRemoved { instance_id },
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
                ServerMessage::GroundItemRemoved { instance_id: id },
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
}

use super::*;

/// Where a carried item sits.
pub enum Carried {
    Worn(onlinerpg_shared::inventory::EquipSlot),
    InBag(u64),
}

/// Result of looking up every bag copy of a named item, for actions that can
/// request more than one unit (sell/drop with a qty). See
/// `AgentState::find_carried_bag_copies`.
pub enum CarriedBagCopies {
    /// At least one bag copy has unspent quantity this turn. `copies` is
    /// (instance_id, remaining_qty) pairs — several entries when the same
    /// item_def_id is fragmented across separate stacks or individually
    /// picked-up non-stackable copies.
    InBag {
        def_id: String,
        copies: Vec<(u64, u32)>,
    },
    /// Known only as an equipped item — no bag copy to sell/drop.
    WornOnly { def_id: String },
}

impl SharedState {
    /// Every bag copy of the resolved item still available this turn.
    /// `spent` counts the units of each instance already given away earlier
    /// this turn, keyed by instance id: the bag snapshot only refreshes when
    /// InventoryUpdated arrives, and a stack survives a sale one unit at a
    /// time, so an instance stays sellable until its whole quantity is gone.
    pub fn find_carried_bag_copies(
        &self,
        asked: &str,
        spent: &HashMap<u64, u32>,
    ) -> Option<CarriedBagCopies> {
        let (id, placed) = self.find_carried(asked)?;
        let copies: Vec<(u64, u32)> = self
            .self_bag
            .iter()
            .filter(|i| i.item_def_id == id)
            .filter_map(|i| {
                let already = spent.get(&i.instance_id).copied().unwrap_or(0);
                let remaining = i.quantity.saturating_sub(already);
                (remaining > 0).then_some((i.instance_id, remaining))
            })
            .collect();
        if !copies.is_empty() {
            return Some(CarriedBagCopies::InBag { def_id: id, copies });
        }
        match placed {
            Carried::Worn(_) => Some(CarriedBagCopies::WornOnly { def_id: id }),
            // Every bag copy was already spent this turn — same outcome as
            // not finding it at all.
            Carried::InBag(_) => None,
        }
    }

    /// Whether we carry `item_def_id`, bag or worn — the server's own test.
    pub fn holds_item(&self, item_def_id: &str) -> bool {
        self.self_bag
            .iter()
            .chain(self.self_equipped.values())
            .any(|i| i.item_def_id == item_def_id)
    }

    /// Find the item the agent named among the ones we carry, and where it
    /// sits. Matching is forgiving about the exact id (see
    /// `item_defs::resolve_named`) but never reaches past what we hold.
    pub fn find_carried(&self, asked: &str) -> Option<(String, Carried)> {
        let ids: Vec<&str> = self
            .self_bag
            .iter()
            .chain(self.self_equipped.values())
            .map(|i| i.item_def_id.as_str())
            .collect();
        let id = crate::item_defs::resolve_named(&ids, asked)?;
        let placed = self
            .self_equipped
            .iter()
            .find(|(_, i)| i.item_def_id == id)
            .map(|(slot, _)| Carried::Worn(*slot))
            .or_else(|| {
                self.self_bag
                    .iter()
                    .find(|i| i.item_def_id == id)
                    .map(|i| Carried::InBag(i.instance_id))
            })?;
        Some((id.to_string(), placed))
    }
}

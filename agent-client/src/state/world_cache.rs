use super::*;

/// Shared world data: passability cache, house state and the generated
/// dungeons. Wrapped in `Arc<RwLock<WorldCache>>` so multiple NPC connections
/// share one copy.
pub struct WorldCache {
    passability_cache: PassabilityCache,
    houses: HashMap<String, HouseData>,
    dungeons: Vec<Arc<Dungeon>>,
    /// Open interior doors and broken props per (entrance id, depth), mirrored
    /// from the server so our A* sees the same walls its movement sim does.
    dungeon_doors: HashMap<(String, u8), HashSet<u32>>,
    dungeon_broken_props: HashMap<(String, u8), Vec<u32>>,
    /// Chest props already opened, per (entrance id, depth). No passability
    /// bearing — an open chest stays solid — but an opened chest is one the
    /// agent should stop seeing, the way its lid stays up for a web player.
    dungeon_opened_props: HashMap<(String, u8), HashSet<u32>>,
    /// Housing chunks and furniture regions already fetched. A chunk that
    /// answered with no houses is indistinguishable from an unfetched one, so
    /// this cannot be derived from `houses`. Shared like the rest of the
    /// cache: what one agent fetched, none of the others ask for again.
    fetched_house_chunks: HashSet<(i32, i32)>,
    fetched_furniture_regions: HashSet<(i32, i32)>,
    /// Raw placements per region, kept so interactions can be resolved back
    /// to the furniture piece (a seated guest's broadcast position is where
    /// they stood when they clicked the chair, not the chair itself).
    furniture_placements: HashMap<(i32, i32), Vec<FurniturePlacement>>,
}

impl WorldCache {
    pub fn new() -> Self {
        Self {
            passability_cache: PassabilityCache::new(),
            houses: HashMap::new(),
            dungeons: Vec::new(),
            dungeon_doors: HashMap::new(),
            dungeon_broken_props: HashMap::new(),
            dungeon_opened_props: HashMap::new(),
            fetched_house_chunks: HashSet::new(),
            fetched_furniture_regions: HashSet::new(),
            furniture_placements: HashMap::new(),
        }
    }

    /// Generate every registry dungeon and register its passability — stair
    /// shafts included, so the shared A* walks from the surface down to the
    /// deepest floor with no extra machinery. Run once at startup, mirroring
    /// the server's own `init_passability`; the entries also give surface
    /// paths the entrance walls the server already collides against.
    pub fn register_dungeons(&mut self) {
        for dungeon in crate::dungeon::build_all() {
            self.passability_cache
                .insert(dungeon_cache_key(&dungeon.id), dungeon.passability());
            self.dungeons.push(Arc::new(dungeon));
        }
    }

    /// Dungeon whose footprint covers (x, z), by the shared registry's
    /// footprint test — the same one the server admits us underground by.
    pub fn dungeon_at(&self, x: f32, z: f32) -> Option<Arc<Dungeon>> {
        let def = onlinerpg_shared::dungeon::entrance_at(x, z)?;
        self.dungeon_by_id(&def.id)
    }

    /// Dungeon with the closest entrance.
    pub fn nearest_dungeon(&self, x: f32, z: f32) -> Option<Arc<Dungeon>> {
        self.dungeons
            .iter()
            .min_by(|a, b| {
                let da = crate::geom::PlanarDelta::xz(x, z, a.entrance.x, a.entrance.z).dist;
                let db = crate::geom::PlanarDelta::xz(x, z, b.entrance.x, b.entrance.z).dist;
                da.total_cmp(&db)
            })
            .map(Arc::clone)
    }

    pub fn dungeon_by_id(&self, id: &str) -> Option<Arc<Dungeon>> {
        self.dungeons.iter().find(|d| d.id == id).map(Arc::clone)
    }

    /// Every registered dungeon: the watch panel draws their entrances, and
    /// name resolution and the world-state listing read it.
    pub fn all_dungeons(&self) -> &[Arc<Dungeon>] {
        &self.dungeons
    }

    pub fn open_dungeon_doors(&self, id: &str, depth: u8) -> HashSet<u32> {
        self.dungeon_doors
            .get(&(id.to_string(), depth))
            .cloned()
            .unwrap_or_default()
    }

    /// Replace the open-door set for a dungeon (the `DungeonDoorsState`
    /// snapshot covers every depth at once, so unlisted floors are all shut).
    pub fn set_dungeon_doors(&mut self, id: &str, doors: &[(u8, u32)]) {
        let touched: HashSet<u8> = self
            .dungeon_doors
            .keys()
            .filter(|(k, _)| k == id)
            .map(|(_, depth)| *depth)
            .chain(doors.iter().map(|(depth, _)| *depth))
            .collect();
        for depth in &touched {
            self.dungeon_doors.remove(&(id.to_string(), *depth));
        }
        for (depth, door_id) in doors {
            self.dungeon_doors
                .entry((id.to_string(), *depth))
                .or_default()
                .insert(*door_id);
        }
        for depth in touched {
            self.rebuild_dungeon_floor(id, depth);
        }
    }

    pub fn set_dungeon_door(&mut self, id: &str, depth: u8, door_id: u32, is_open: bool) {
        let set = self
            .dungeon_doors
            .entry((id.to_string(), depth))
            .or_default();
        // Re-broadcasts are common; rebuilding a floor's 6400 cells under the
        // shared write lock for a state we already hold is not worth it.
        let changed = if is_open {
            set.insert(door_id)
        } else {
            set.remove(&door_id)
        };
        if changed {
            self.rebuild_dungeon_floor(id, depth);
        }
    }

    pub fn set_dungeon_broken_props(&mut self, id: &str, depth: u8, broken: Vec<u32>) {
        let key = (id.to_string(), depth);
        if self.dungeon_broken_props.get(&key) == Some(&broken) {
            return;
        }
        self.dungeon_broken_props.insert(key, broken);
        self.rebuild_dungeon_floor(id, depth);
    }

    pub fn add_dungeon_broken_prop(&mut self, id: &str, depth: u8, prop_id: u32) {
        let broken = self
            .dungeon_broken_props
            .entry((id.to_string(), depth))
            .or_default();
        if broken.contains(&prop_id) {
            return;
        }
        broken.push(prop_id);
        self.rebuild_dungeon_floor(id, depth);
    }

    pub fn set_dungeon_opened_props(&mut self, id: &str, depth: u8, opened: Vec<u32>) {
        self.dungeon_opened_props
            .insert((id.to_string(), depth), opened.into_iter().collect());
    }

    pub fn add_dungeon_opened_prop(&mut self, id: &str, depth: u8, prop_id: u32) {
        self.dungeon_opened_props
            .entry((id.to_string(), depth))
            .or_default()
            .insert(prop_id);
    }

    pub fn remove_dungeon_opened_prop(&mut self, id: &str, depth: u8, prop_id: u32) {
        if let Some(opened) = self.dungeon_opened_props.get_mut(&(id.to_string(), depth)) {
            opened.remove(&prop_id);
        }
    }

    pub fn opened_dungeon_props(&self, id: &str, depth: u8) -> Option<&HashSet<u32>> {
        self.dungeon_opened_props.get(&(id.to_string(), depth))
    }

    /// Broken prop ids for one dungeon floor — `break_prop` checks this before
    /// walking out to a barrel someone already smashed.
    pub fn dungeon_broken_props(&self, id: &str, depth: u8) -> &[u32] {
        self.dungeon_broken_props
            .get(&(id.to_string(), depth))
            .map_or(&[], Vec::as_slice)
    }

    /// Whether a mover can stand in the cell holding `(x, z)` on `floor`. What
    /// the in-room sighting queries use to decide where a prop can be opened
    /// from.
    pub fn is_walkable(&self, x: f32, z: f32, floor: u8) -> bool {
        !pathfinding::is_cell_sealed(self.passability_cache(), x, z, floor, None)
    }

    /// Recompute one dungeon floor's cells from the live door/prop state
    /// (shared `dungeon::floor_cells`).
    fn rebuild_dungeon_floor(&mut self, id: &str, depth: u8) {
        let Some(dungeon) = self.dungeon_by_id(id) else {
            return;
        };
        let open = self.open_dungeon_doors(id, depth);
        let broken = self
            .dungeon_broken_props
            .get(&(id.to_string(), depth))
            .cloned()
            .unwrap_or_default();
        let Some(cells) = floor_cells(dungeon.layouts(), depth, &broken, Some(&open)) else {
            return;
        };
        set_floor_cells(&mut self.passability_cache, id, depth, cells);
    }

    pub fn passability_cache(&self) -> &PassabilityCache {
        &self.passability_cache
    }

    pub fn houses(&self) -> &HashMap<String, HouseData> {
        &self.houses
    }

    pub fn add_house(&mut self, house: HouseData) {
        let rp = pathfinding::build_runtime_passability(&house);
        self.passability_cache.insert(house.id.clone(), rp);
        pathfinding::apply_door_overlays(&mut self.passability_cache, &house);
        self.houses.insert(house.id.clone(), house);
    }

    pub fn remove_house(&mut self, house_id: &str) {
        self.houses.remove(house_id);
        self.passability_cache.remove(house_id);
    }

    /// Chunks/regions of `wanted` nobody has fetched yet. Only chunks that
    /// answered are marked, so a failed one comes back on the next ask.
    pub fn unfetched_house_chunks(&self, wanted: &mut HashSet<(i32, i32)>) {
        wanted.retain(|c| !self.fetched_house_chunks.contains(c));
    }

    pub fn mark_houses_fetched(&mut self, chunk: (i32, i32)) {
        self.fetched_house_chunks.insert(chunk);
    }

    pub fn unfetched_furniture_regions(&self, wanted: &mut HashSet<(i32, i32)>) {
        wanted.retain(|r| !self.fetched_furniture_regions.contains(r));
    }

    pub fn mark_furniture_fetched(&mut self, region: (i32, i32)) {
        self.fetched_furniture_regions.insert(region);
    }

    /// Register (or replace) a region's solid furniture in the passability cache
    /// so the bot paths around it, mirroring the browser's
    /// `passability_set_furniture` (same `furniture:rx,rz` key + shared
    /// `furniture` resolution). Empty/non-solid regions clear the entry.
    pub fn sync_furniture(&mut self, rx: i32, rz: i32, placements: Vec<FurniturePlacement>) {
        let key = furniture::region_cache_key(rx, rz);
        match furniture::build_furniture_passability_for_placements(&placements) {
            Some(rp) => {
                self.passability_cache.insert(key, rp);
            }
            None => {
                self.passability_cache.remove(&key);
            }
        }
        self.furniture_placements.insert((rx, rz), placements);
    }

    /// The `type_id` placement with editor id `object_id` nearest `(x, z)`,
    /// within `max_dist`. Ids are unique only within their region file, so
    /// the type and distance bound are what disambiguate.
    pub fn furniture_placement_near(
        &self,
        type_id: &str,
        object_id: u32,
        x: f32,
        z: f32,
        max_dist: f32,
    ) -> Option<&FurniturePlacement> {
        let max_d2 = max_dist * max_dist;
        let d2 = |p: &FurniturePlacement| (p.x - x).powi(2) + (p.z - z).powi(2);
        self.furniture_placements
            .values()
            .flatten()
            .filter(|p| p.id == object_id && p.type_id == type_id && d2(p) <= max_d2)
            .min_by(|a, b| d2(a).total_cmp(&d2(b)))
    }

    pub fn update_door(
        &mut self,
        house_id: &str,
        room_index: u32,
        wall_dir: WallDirection,
        segment_index: usize,
        is_open: bool,
    ) {
        if let Some(house) = self.houses.get_mut(house_id) {
            if let Some(room) = house.rooms.get_mut(room_index as usize) {
                // The wall is the source of truth (door hunting reads
                // `is_open` off it); the edge is derived from it.
                if let Some(wall) = room.wall_mut(wall_dir).get_mut(segment_index) {
                    wall.is_open = is_open;
                    pathfinding::update_door_edge(
                        &mut self.passability_cache,
                        house_id,
                        room,
                        wall_dir,
                        segment_index,
                        is_open,
                    );
                }
            }
        }
    }
}

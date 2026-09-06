use super::{AuthError, AuthService, CharacterSaveData, ItemRow};
use onlinerpg_shared::estate_storage::{estate_storage_def, EstateChest, EstateChestState};
use onlinerpg_shared::fence::FencePlot;
use onlinerpg_shared::inventory::ItemInstance;
use onlinerpg_shared::messages::BagLineItem;
use onlinerpg_shared::Position;
use onlinerpg_terrain::land::plot_addr;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

pub(crate) struct EstateDeposit {
    pub item: ItemInstance,
    pub quantity: u32,
    pub stackable: bool,
}

fn plot_key(x: f32, z: f32) -> (i32, i32, u8) {
    let addr = plot_addr(x, z);
    let tile = (addr.index / 4) as i32;
    (
        addr.rx * 16 + tile % 16,
        addr.rz * 16 + tile / 16,
        (addr.index % 4) as u8,
    )
}

fn active_estate_at(
    tx: &Transaction<'_>,
    owner_id: i64,
    x: f32,
    z: f32,
) -> Result<Option<i64>, rusqlite::Error> {
    let (tile_x, tile_z, quadrant) = plot_key(x, z);
    tx.query_row(
        "SELECT e.id FROM land_plots p JOIN land_estates e ON e.id=p.estate_id
         WHERE p.tile_x=?1 AND p.tile_z=?2 AND p.quadrant=?3
           AND e.owner_id=?4 AND e.missed=0",
        params![tile_x, tile_z, quadrant, owner_id],
        |row| row.get(0),
    )
    .optional()
}

fn chest_access(
    tx: &Transaction<'_>,
    chest_id: i64,
    character_id: i64,
) -> Result<Option<(u64, bool)>, rusqlite::Error> {
    tx.query_row(
        "SELECT c.revision, c.owner_id=?2 AND COALESCE(e.missed, 1)=0
         FROM estate_chests c LEFT JOIN land_estates e ON e.id=c.estate_id
         WHERE c.id=?1 AND (c.owner_id=?2 OR e.id IS NULL)",
        params![chest_id, character_id],
        |row| Ok((row.get::<_, i64>(0)? as u64, row.get(1)?)),
    )
    .optional()
}

fn read_items(conn: &Connection, chest_id: i64) -> Result<Vec<ItemInstance>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id,item_def_id,quantity,enchant,cape_color,cape_texture
         FROM estate_chest_items WHERE chest_id=?1 ORDER BY id",
    )?;
    let items = stmt
        .query_map([chest_id], |row| {
            Ok(ItemInstance {
                instance_id: row.get::<_, i64>(0)? as u64,
                item_def_id: row.get(1)?,
                quantity: row.get::<_, i64>(2)? as u32,
                enchant: row.get(3)?,
                cape_color: row.get(4)?,
                cape_texture: row.get(5)?,
            })
        })?
        .collect();
    items
}

impl AuthService {
    pub(super) fn ensure_estate_storage_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS estate_chests (
                id INTEGER PRIMARY KEY,
                estate_id INTEGER NOT NULL,
                owner_id INTEGER NOT NULL,
                item_def_id TEXT NOT NULL DEFAULT 'storage_chest',
                x REAL NOT NULL, y REAL NOT NULL, z REAL NOT NULL,
                rotation_deg REAL NOT NULL,
                floor_level INTEGER NOT NULL,
                revision INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (owner_id) REFERENCES characters(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS estate_chest_items (
                id INTEGER PRIMARY KEY,
                chest_id INTEGER NOT NULL REFERENCES estate_chests(id) ON DELETE CASCADE,
                item_def_id TEXT NOT NULL,
                quantity INTEGER NOT NULL CHECK (quantity > 0),
                enchant INTEGER NOT NULL DEFAULT 0,
                cape_color TEXT,
                cape_texture TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_estate_chest_items_chest
                ON estate_chest_items(chest_id);",
        )?;
        if !Self::table_columns(conn, "estate_chests")?.contains("item_def_id") {
            conn.execute(
                "ALTER TABLE estate_chests
                 ADD COLUMN item_def_id TEXT NOT NULL DEFAULT 'storage_chest'",
                [],
            )?;
        }
        let schema: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='estate_chests'",
            [],
            |row| row.get(0),
        )?;
        if schema.contains("estate_id INTEGER NOT NULL UNIQUE") {
            let foreign_keys: bool = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
            conn.pragma_update(None, "foreign_keys", false)?;
            let migration = conn.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE estate_chests_new (
                    id INTEGER PRIMARY KEY,
                    estate_id INTEGER NOT NULL,
                    owner_id INTEGER NOT NULL,
                    item_def_id TEXT NOT NULL DEFAULT 'storage_chest',
                    x REAL NOT NULL, y REAL NOT NULL, z REAL NOT NULL,
                    rotation_deg REAL NOT NULL,
                    floor_level INTEGER NOT NULL,
                    revision INTEGER NOT NULL DEFAULT 0,
                    FOREIGN KEY (owner_id) REFERENCES characters(id) ON DELETE CASCADE
                 );
                 INSERT INTO estate_chests_new
                    (id,estate_id,owner_id,item_def_id,x,y,z,rotation_deg,floor_level,revision)
                    SELECT id,estate_id,owner_id,item_def_id,x,y,z,rotation_deg,floor_level,revision
                    FROM estate_chests;
                 CREATE TABLE estate_chest_items_new (
                    id INTEGER PRIMARY KEY,
                    chest_id INTEGER NOT NULL REFERENCES estate_chests(id) ON DELETE CASCADE,
                    item_def_id TEXT NOT NULL,
                    quantity INTEGER NOT NULL CHECK (quantity > 0),
                    enchant INTEGER NOT NULL DEFAULT 0,
                    cape_color TEXT,
                    cape_texture TEXT
                 );
                 INSERT INTO estate_chest_items_new
                    (id,chest_id,item_def_id,quantity,enchant,cape_color,cape_texture)
                    SELECT id,chest_id,item_def_id,quantity,enchant,cape_color,cape_texture
                    FROM estate_chest_items;
                 DROP TABLE estate_chest_items;
                 DROP TABLE estate_chests;
                 ALTER TABLE estate_chests_new RENAME TO estate_chests;
                 ALTER TABLE estate_chest_items_new RENAME TO estate_chest_items;
                 CREATE INDEX idx_estate_chest_items_chest
                    ON estate_chest_items(chest_id);
                 COMMIT;",
            );
            if migration.is_err() {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            conn.pragma_update(None, "foreign_keys", foreign_keys)?;
            migration?;
        }
        Ok(())
    }

    pub fn load_estate_chests(&self) -> Result<Vec<EstateChest>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT c.id,c.estate_id,c.owner_id,c.item_def_id,c.x,c.y,c.z,c.rotation_deg,c.floor_level,
                    COALESCE(e.missed,1)>0,c.revision
             FROM estate_chests c LEFT JOIN land_estates e ON e.id=c.estate_id",
        )?;
        let chests = stmt
            .query_map([], |row| {
                Ok(EstateChest {
                    id: row.get(0)?,
                    estate_id: row.get(1)?,
                    owner_id: row.get(2)?,
                    item_def_id: row.get(3)?,
                    position: Position {
                        x: row.get::<_, f64>(4)? as f32,
                        y: row.get::<_, f64>(5)? as f32,
                        z: row.get::<_, f64>(6)? as f32,
                    },
                    rotation_deg: row.get::<_, f64>(7)? as f32,
                    floor_level: row.get::<_, i64>(8)? as i8,
                    overdue: row.get(9)?,
                    revision: row.get::<_, i64>(10)? as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(chests)
    }

    pub fn estate_storage_plots(&self, character_id: i64) -> Result<Vec<FencePlot>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT p.tile_x,p.tile_z,p.quadrant FROM land_plots p
             JOIN land_estates e ON e.id=p.estate_id
             WHERE e.owner_id=?1 AND e.missed=0",
        )?;
        let plots = stmt
            .query_map([character_id], |row| {
                let tile_x: i32 = row.get(0)?;
                let tile_z: i32 = row.get(1)?;
                let quadrant: i32 = row.get(2)?;
                Ok(FencePlot {
                    x: tile_x * 64 - 32 + (quadrant % 2) * 32,
                    z: tile_z * 64 - 32 + (quadrant / 2) * 32,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(plots)
    }

    pub fn place_estate_chest(
        &self,
        character: &CharacterSaveData,
        inventory: &[ItemRow],
        item_def_id: String,
        position: Position,
        rotation_deg: f32,
        floor_level: i8,
    ) -> Result<Result<EstateChest, &'static str>, AuthError> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(definition) = estate_storage_def(&item_def_id) else {
            return Ok(Err("That item is not estate storage."));
        };
        let radians = rotation_deg.to_radians();
        let half_x = (definition.footprint_width * radians.cos().abs()
            + definition.footprint_depth * radians.sin().abs())
            / 2.0;
        let half_z = (definition.footprint_width * radians.sin().abs()
            + definition.footprint_depth * radians.cos().abs())
            / 2.0;
        let mut estate_id = None;
        for (dx, dz) in [
            (0.0, 0.0),
            (-half_x, -half_z),
            (-half_x, half_z),
            (half_x, -half_z),
            (half_x, half_z),
        ] {
            let found = active_estate_at(
                &tx,
                character.character_id,
                position.x + dx,
                position.z + dz,
            )?;
            if found.is_none() || estate_id.is_some_and(|id| Some(id) != found) {
                return Ok(Err("The whole chest must be inside your active estate."));
            }
            estate_id = found;
        }
        let estate_id = estate_id.expect("center sample always sets estate");
        tx.execute(
            "INSERT INTO estate_chests
             (estate_id,owner_id,item_def_id,x,y,z,rotation_deg,floor_level)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                estate_id,
                character.character_id,
                item_def_id,
                position.x,
                position.y,
                position.z,
                rotation_deg,
                floor_level
            ],
        )?;
        let id = tx.last_insert_rowid();
        Self::write_character_states(&tx, std::slice::from_ref(character))?;
        Self::replace_inventories(&tx, [(character.character_id, inventory)])?;
        tx.commit()?;
        Ok(Ok(EstateChest {
            id,
            estate_id,
            owner_id: character.character_id,
            item_def_id,
            position,
            rotation_deg,
            floor_level,
            overdue: false,
            revision: 0,
        }))
    }

    pub fn estate_chest_state(
        &self,
        chest_id: i64,
        character_id: i64,
    ) -> Result<Result<EstateChestState, &'static str>, AuthError> {
        let conn = self.open_connection()?;
        let access: Option<(u64, bool, String)> = conn
            .query_row(
                "SELECT c.revision,c.owner_id=?2 AND COALESCE(e.missed,1)=0,c.item_def_id
                 FROM estate_chests c
                 LEFT JOIN land_estates e ON e.id=c.estate_id
                 WHERE c.id=?1 AND (c.owner_id=?2 OR e.id IS NULL)",
                params![chest_id, character_id],
                |row| Ok((row.get::<_, i64>(0)? as u64, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((revision, can_deposit, item_def_id)) = access else {
            return Ok(Err(
                "You do not own this storage chest, and it is not abandoned.",
            ));
        };
        let Some(definition) = estate_storage_def(&item_def_id) else {
            return Ok(Err("This storage chest has an unknown type."));
        };
        Ok(Ok(EstateChestState {
            chest_id,
            item_def_id,
            revision,
            max_weight: definition.max_weight(),
            can_deposit,
            items: read_items(&conn, chest_id)?,
        }))
    }

    pub fn transfer_estate_items(
        &self,
        character: &CharacterSaveData,
        inventory: &[ItemRow],
        chest_id: i64,
        deposits: &[EstateDeposit],
        withdrawals: &[BagLineItem],
        expected_revision: u64,
    ) -> Result<Result<u64, &'static str>, AuthError> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((revision, active)) = chest_access(&tx, chest_id, character.character_id)? else {
            return Ok(Err(
                "You do not own this storage chest, and it is not abandoned.",
            ));
        };
        if !deposits.is_empty() && !active {
            return Ok(Err(
                "Overdue estates can withdraw items but cannot store new ones.",
            ));
        }
        if revision != expected_revision {
            return Ok(Err("The chest changed. Its contents were refreshed."));
        }
        for withdrawal in withdrawals {
            let stored: Option<u32> = tx
                .query_row(
                    "SELECT quantity FROM estate_chest_items WHERE id=?1 AND chest_id=?2",
                    params![withdrawal.instance_id as i64, chest_id],
                    |row| Ok(row.get::<_, i64>(0)? as u32),
                )
                .optional()?;
            let Some(stored) = stored else {
                return Ok(Err("An item is no longer in the chest."));
            };
            if withdrawal.qty == 0 || withdrawal.qty > stored {
                return Ok(Err("Invalid storage quantity."));
            }
            if withdrawal.qty == stored {
                tx.execute(
                    "DELETE FROM estate_chest_items WHERE id=?1",
                    [withdrawal.instance_id as i64],
                )?;
            } else {
                tx.execute(
                    "UPDATE estate_chest_items SET quantity=quantity-?2 WHERE id=?1",
                    params![withdrawal.instance_id as i64, withdrawal.qty],
                )?;
            }
        }
        for deposit in deposits {
            let item = &deposit.item;
            let matching: Option<i64> = if deposit.stackable {
                tx.query_row(
                    "SELECT id FROM estate_chest_items WHERE chest_id=?1 AND item_def_id=?2
                     AND enchant=?3 AND cape_color IS ?4 AND cape_texture IS ?5 LIMIT 1",
                    params![
                        chest_id,
                        item.item_def_id,
                        item.enchant,
                        item.cape_color,
                        item.cape_texture
                    ],
                    |row| row.get(0),
                )
                .optional()?
            } else {
                None
            };
            if let Some(id) = matching {
                let changed = tx.execute(
                    "UPDATE estate_chest_items SET quantity=quantity+?2
                     WHERE id=?1 AND quantity<=?3",
                    params![id, deposit.quantity, i64::from(u32::MAX - deposit.quantity)],
                )?;
                if changed == 0 {
                    return Ok(Err("A stored stack is full."));
                }
            } else {
                tx.execute(
                    "INSERT INTO estate_chest_items
                     (chest_id,item_def_id,quantity,enchant,cape_color,cape_texture)
                     VALUES (?1,?2,?3,?4,?5,?6)",
                    params![
                        chest_id,
                        item.item_def_id,
                        deposit.quantity,
                        item.enchant,
                        item.cape_color,
                        item.cape_texture
                    ],
                )?;
            }
        }
        let next = revision + 1;
        tx.execute(
            "UPDATE estate_chests SET revision=?2 WHERE id=?1",
            params![chest_id, next],
        )?;
        Self::write_character_states(&tx, std::slice::from_ref(character))?;
        Self::replace_inventories(&tx, [(character.character_id, inventory)])?;
        tx.commit()?;
        Ok(Ok(next))
    }

    pub fn recover_estate_chest(
        &self,
        character: &CharacterSaveData,
        inventory: &[ItemRow],
        chest_id: i64,
    ) -> Result<Result<(), &'static str>, AuthError> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if chest_access(&tx, chest_id, character.character_id)?.is_none() {
            return Ok(Err(
                "You can only recover your own or an abandoned storage chest.",
            ));
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM estate_chest_items WHERE chest_id=?1",
            [chest_id],
            |row| row.get(0),
        )?;
        if count != 0 {
            return Ok(Err("Empty the storage chest before recovering it."));
        }
        tx.execute("DELETE FROM estate_chests WHERE id=?1", [chest_id])?;
        Self::write_character_states(&tx, std::slice::from_ref(character))?;
        Self::replace_inventories(&tx, [(character.character_id, inventory)])?;
        tx.commit()?;
        Ok(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_migration_preserves_contents_and_allows_multiple_chests() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE characters (id INTEGER PRIMARY KEY);
             INSERT INTO characters (id) VALUES (1);
             CREATE TABLE estate_chests (
                id INTEGER PRIMARY KEY,
                estate_id INTEGER NOT NULL UNIQUE,
                owner_id INTEGER NOT NULL,
                x REAL NOT NULL, y REAL NOT NULL, z REAL NOT NULL,
                rotation_deg REAL NOT NULL,
                floor_level INTEGER NOT NULL,
                revision INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (owner_id) REFERENCES characters(id) ON DELETE CASCADE
             );
             CREATE TABLE estate_chest_items (
                id INTEGER PRIMARY KEY,
                chest_id INTEGER NOT NULL REFERENCES estate_chests(id) ON DELETE CASCADE,
                item_def_id TEXT NOT NULL,
                quantity INTEGER NOT NULL CHECK (quantity > 0),
                enchant INTEGER NOT NULL DEFAULT 0,
                cape_color TEXT,
                cape_texture TEXT
             );
             INSERT INTO estate_chests
                (id,estate_id,owner_id,x,y,z,rotation_deg,floor_level)
                VALUES (10,7,1,1.0,2.0,3.0,0.0,0);
             INSERT INTO estate_chest_items
                (id,chest_id,item_def_id,quantity)
                VALUES (20,10,'apple',3);",
        )
        .unwrap();

        AuthService::ensure_estate_storage_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO estate_chests
             (estate_id,owner_id,x,y,z,rotation_deg,floor_level)
             VALUES (7,1,4.0,2.0,5.0,90.0,0)",
            [],
        )
        .unwrap();

        let chest_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM estate_chests", [], |row| row.get(0))
            .unwrap();
        let item_quantity: i64 = conn
            .query_row(
                "SELECT quantity FROM estate_chest_items WHERE id=20 AND chest_id=10",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let item_def_id: String = conn
            .query_row(
                "SELECT item_def_id FROM estate_chests WHERE id=10",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let foreign_keys: bool = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(chest_count, 2);
        assert_eq!(item_quantity, 3);
        assert_eq!(item_def_id, "storage_chest");
        assert!(foreign_keys);
    }
}

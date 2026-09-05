use super::{AuthError, AuthService, CharacterSaveData, ItemRow};
use onlinerpg_shared::fence::{Fence, FenceAxis, FenceEdge, FencePlot};
use onlinerpg_terrain::land::plot_addr;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

pub struct FenceRecord {
    pub edge: FenceEdge,
    pub owner_id: i64,
}

fn axis_id(axis: FenceAxis) -> u8 {
    match axis {
        FenceAxis::X => 0,
        FenceAxis::Z => 1,
    }
}

impl AuthService {
    pub(super) fn ensure_fence_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS land_fences (
                x INTEGER NOT NULL, z INTEGER NOT NULL,
                axis INTEGER NOT NULL CHECK (axis IN (0, 1)),
                estate_id INTEGER NOT NULL REFERENCES land_estates(id) ON DELETE CASCADE,
                PRIMARY KEY (x, z, axis)
            );",
        )?;
        for column in ["y", "owner_id"] {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('land_fences') WHERE name=?1)",
                [column],
                |row| row.get(0),
            )?;
            if exists {
                tx.execute_batch(&format!("ALTER TABLE land_fences DROP COLUMN {column};"))?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_fences(&self) -> Result<Vec<FenceRecord>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT f.x, f.z, f.axis, e.owner_id FROM land_fences f
             JOIN land_estates e ON e.id=f.estate_id",
        )?;
        let fences = stmt
            .query_map([], |row| {
                Ok(FenceRecord {
                    edge: FenceEdge {
                        x: row.get(0)?,
                        z: row.get(1)?,
                        axis: if row.get::<_, u8>(2)? == 0 {
                            FenceAxis::X
                        } else {
                            FenceAxis::Z
                        },
                    },
                    owner_id: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(fences)
    }

    pub fn fence_plots(&self, character_id: i64) -> Result<Vec<FencePlot>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT p.tile_x, p.tile_z, p.quadrant FROM land_plots p
             JOIN land_estates e ON e.id=p.estate_id WHERE e.owner_id=?1 AND e.missed=0",
        )?;
        let plots = stmt
            .query_map([character_id], |row| {
                let tx: i32 = row.get(0)?;
                let tz: i32 = row.get(1)?;
                let q: i32 = row.get(2)?;
                Ok(FencePlot {
                    x: tx * 64 - 32 + (q % 2) * 32,
                    z: tz * 64 - 32 + (q / 2) * 32,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(plots)
    }

    pub fn save_fence_edit(
        &self,
        character: &CharacterSaveData,
        inventory: &[ItemRow],
        fence: &Fence,
        place: bool,
    ) -> Result<Result<(), &'static str>, AuthError> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let edge = fence.edge;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT e.owner_id FROM land_fences f JOIN land_estates e ON e.id=f.estate_id
                 WHERE f.x=?1 AND f.z=?2 AND f.axis=?3",
                params![edge.x, edge.z, axis_id(edge.axis)],
                |row| row.get(0),
            )
            .optional()?;
        if place {
            if existing.is_some() {
                return Ok(Err("A fence is already on that edge."));
            }
            let mut estate = None;
            for center in edge.adjacent_centers() {
                let addr = plot_addr(center.x, center.z);
                let tile = (addr.index / 4) as i32;
                let tx_coord = addr.rx * 16 + tile % 16;
                let tz_coord = addr.rz * 16 + tile / 16;
                estate = tx.query_row(
                    "SELECT e.id FROM land_plots p JOIN land_estates e ON e.id=p.estate_id
                     WHERE p.tile_x=?1 AND p.tile_z=?2 AND p.quadrant=?3 AND e.owner_id=?4 AND e.missed=0",
                    params![tx_coord, tz_coord, addr.index % 4, character.character_id], |row| row.get::<_, i64>(0)
                ).optional()?;
                if estate.is_some() {
                    break;
                }
            }
            let Some(estate_id) = estate else {
                return Ok(Err(
                    "Place fences on your own estate. Overdue estates cannot add fences.",
                ));
            };
            tx.execute(
                "INSERT INTO land_fences (x,z,axis,estate_id) VALUES (?1,?2,?3,?4)",
                params![edge.x, edge.z, axis_id(edge.axis), estate_id],
            )?;
        } else {
            match existing {
                None => return Ok(Err("That fence has already been removed.")),
                Some(owner) if owner != character.character_id => {
                    return Ok(Err("You can only recover your own fences."))
                }
                _ => {}
            }
            tx.execute(
                "DELETE FROM land_fences WHERE x=?1 AND z=?2 AND axis=?3",
                params![edge.x, edge.z, axis_id(edge.axis)],
            )?;
        }
        Self::write_character_states(&tx, std::slice::from_ref(character))?;
        Self::replace_inventories(&tx, [(character.character_id, inventory)])?;
        tx.commit()?;
        Ok(Ok(()))
    }
}

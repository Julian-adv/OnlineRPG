use super::{AuthError, AuthService, CharacterSaveData, ItemRow};
use onlinerpg_terrain::coords::{tile_to_region, WORLD_TILES_X};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;

const WORLD_PLOTS_X: i32 = WORLD_TILES_X * 2;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedLandPlot {
    pub rx: i32,
    pub rz: i32,
    pub index: usize,
    pub owner_name: String,
}

impl AuthService {
    pub fn owned_land_plots(&self) -> Result<Vec<OwnedLandPlot>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT p.tile_x, p.tile_z, p.quadrant, c.character_name
             FROM land_plots p
             JOIN land_estates e ON e.id=p.estate_id
             JOIN characters c ON c.id=e.owner_id",
        )?;
        let plots = stmt.query_map([], |row| {
            let tx: i32 = row.get(0)?;
            let tz: i32 = row.get(1)?;
            let quadrant: usize = row.get(2)?;
            Ok(OwnedLandPlot {
                rx: tile_to_region(tx),
                rz: tile_to_region(tz),
                index: (tz.rem_euclid(16) * 16 + tx.rem_euclid(16)) as usize * 4 + quadrant,
                owner_name: row.get(3)?,
            })
        })?;
        Ok(plots.collect::<Result<Vec<_>, _>>()?)
    }

    pub(super) fn ensure_land_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS land_estates (
                id INTEGER PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
                account_name TEXT NOT NULL REFERENCES accounts(player_name) ON DELETE CASCADE,
                name TEXT,
                grade INTEGER NOT NULL CHECK (grade IN (1, 2)),
                source TEXT NOT NULL DEFAULT 'purchase',
                transferable INTEGER NOT NULL DEFAULT 1,
                treasury INTEGER NOT NULL DEFAULT 0,
                missed INTEGER NOT NULL DEFAULT 0,
                free_months INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                UNIQUE (account_name, grade)
            );
            CREATE TABLE IF NOT EXISTS land_plots (
                tile_x INTEGER NOT NULL,
                tile_z INTEGER NOT NULL,
                quadrant INTEGER NOT NULL CHECK (quadrant BETWEEN 0 AND 3),
                estate_id INTEGER NOT NULL REFERENCES land_estates(id) ON DELETE CASCADE,
                PRIMARY KEY (tile_x, tile_z, quadrant)
            );
            CREATE INDEX IF NOT EXISTS idx_land_plots_estate ON land_plots(estate_id);",
        )
    }

    pub fn claim_homestead(
        &self,
        character: &CharacterSaveData,
        plot: (i32, i32, u8),
        inventory: &[ItemRow],
    ) -> Result<Result<i64, &'static str>, AuthError> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let estate_id = match Self::validate_homestead_claim(&tx, character.character_id, plot)? {
            Ok(Some(id)) => id,
            Ok(None) => {
                tx.execute(
                    "INSERT INTO land_estates (owner_id, account_name, grade, created_at) SELECT id, account_name, 1, ?2 FROM characters WHERE id=?1",
                    params![character.character_id, super::unix_now()],
                )?;
                tx.last_insert_rowid()
            }
            Err(reason) => return Ok(Err(reason)),
        };
        let (tile_x, tile_z, quadrant) = plot;
        tx.execute(
            "INSERT INTO land_plots (tile_x, tile_z, quadrant, estate_id) VALUES (?1, ?2, ?3, ?4)",
            params![tile_x, tile_z, quadrant, estate_id],
        )?;
        Self::write_character_states(&tx, std::slice::from_ref(character))?;
        Self::replace_inventories(&tx, [(character.character_id, inventory)])?;
        tx.commit()?;
        Ok(Ok(estate_id))
    }

    pub fn check_homestead_claim(
        &self,
        character_id: i64,
        plot: (i32, i32, u8),
    ) -> Result<Result<(), &'static str>, AuthError> {
        let conn = self.open_connection()?;
        let tx = conn.unchecked_transaction()?;
        Ok(Self::validate_homestead_claim(&tx, character_id, plot)?.map(|_| ()))
    }

    fn validate_homestead_claim(
        conn: &Connection,
        character_id: i64,
        plot: (i32, i32, u8),
    ) -> Result<Result<Option<i64>, &'static str>, AuthError> {
        let (tile_x, tile_z, quadrant) = plot;
        let occupied: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM land_plots WHERE tile_x=?1 AND tile_z=?2 AND quadrant=?3)",
            params![tile_x, tile_z, quadrant],
            |row| row.get(0),
        )?;
        if occupied {
            return Ok(Err("This plot already belongs to someone."));
        }
        let account: String = conn.query_row(
            "SELECT account_name FROM characters WHERE id=?1",
            [character_id],
            |row| row.get(0),
        )?;
        let estate: Option<(i64, i64)> = conn
            .query_row(
                "SELECT id, owner_id FROM land_estates WHERE account_name=?1 AND grade=1",
                [&account],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((estate_id, owner)) = estate else {
            return Ok(Ok(None));
        };
        if owner != character_id {
            return Ok(Err(
                "Another character on your account already owns a homestead.",
            ));
        }
        let mut stmt =
            conn.prepare("SELECT tile_x, tile_z, quadrant FROM land_plots WHERE estate_id=?1")?;
        let plots = stmt
            .query_map([estate_id], |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, u8>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if plots.len() >= 16 {
            return Ok(Err("Your homestead has reached its limit of 16 plots."));
        }
        let cell = |(x, z, q): (i32, i32, u8)| (2 * x + i32::from(q % 2), 2 * z + i32::from(q / 2));
        let (x, z) = cell(plot);
        let offsets: Vec<_> = plots
            .into_iter()
            .map(|p| {
                let (px, pz) = cell(p);
                (
                    (px - x + WORLD_PLOTS_X / 2).rem_euclid(WORLD_PLOTS_X) - WORLD_PLOTS_X / 2,
                    pz - z,
                )
            })
            .collect();
        if !offsets.iter().any(|(dx, dz)| dx.abs() + dz.abs() == 1) {
            return Ok(Err(
                "Choose a plot that shares an edge with your homestead.",
            ));
        }
        let (mut min_x, mut max_x, mut min_z, mut max_z) = (0, 0, 0, 0);
        for (dx, dz) in offsets {
            min_x = min_x.min(dx);
            max_x = max_x.max(dx);
            min_z = min_z.min(dz);
            max_z = max_z.max(dz);
        }
        if max_x - min_x >= 8 || max_z - min_z >= 8 {
            return Ok(Err("Your homestead must fit within an 8 by 8 plot area."));
        }
        Ok(Ok(Some(estate_id)))
    }
}

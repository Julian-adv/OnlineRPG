use super::{AuthError, AuthService, CharacterSaveData, ItemRow};
use onlinerpg_terrain::coords::{tile_to_region, WORLD_TILES_X};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;

const WORLD_PLOTS_X: i32 = WORLD_TILES_X * 2;
pub const LAND_TAX_PER_PLOT: i64 = 2_000;

#[derive(Default, Debug)]
pub struct LandAccount {
    pub treasury: i64,
    pub plots: u32,
    pub missed: u32,
    pub free_months: u32,
}

impl LandAccount {
    pub fn monthly_tax(&self) -> i64 {
        i64::from(self.plots) * LAND_TAX_PER_PLOT
    }

    pub fn recovery_cost(&self) -> i64 {
        if self.missed == 0 {
            0
        } else {
            self.monthly_tax() * (i64::from(self.missed) + 1)
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedLandPlot {
    pub rx: i32,
    pub rz: i32,
    pub index: usize,
    pub owner_name: String,
}

impl AuthService {
    fn read_land_account(conn: &Connection, character_id: i64) -> Result<LandAccount, AuthError> {
        Ok(conn.query_row(
            "SELECT treasury, missed, free_months, (SELECT COUNT(*) FROM land_plots WHERE estate_id=e.id)
             FROM land_estates e WHERE owner_id=?1 AND grade=1",
            [character_id],
            |row| Ok(LandAccount { treasury: row.get(0)?, missed: row.get(1)?, free_months: row.get(2)?, plots: row.get(3)? }),
        ).optional()?.unwrap_or_default())
    }

    pub fn land_account(&self, character_id: i64) -> Result<LandAccount, AuthError> {
        let conn = self.open_connection()?;
        Self::read_land_account(&conn, character_id)
    }

    pub fn transfer_land_gold(
        &self,
        mut character: CharacterSaveData,
        inventory: &[ItemRow],
        amount: i64,
        deposit: bool,
    ) -> Result<Result<(i64, LandAccount), &'static str>, AuthError> {
        if amount <= 0 {
            return Ok(Err("Enter a positive amount."));
        }
        let mut conn = self.open_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut account = Self::read_land_account(&tx, character.character_id)?;
        if account.plots == 0 {
            return Ok(Err("Claim a homestead before using its tax account."));
        }
        let (gold, treasury) = if deposit {
            if amount > character.gold {
                return Ok(Err("You do not have enough gold."));
            }
            (
                Some(character.gold - amount),
                account.treasury.checked_add(amount),
            )
        } else {
            if amount > account.treasury {
                return Ok(Err("Your tax account does not have enough gold."));
            }
            (
                character.gold.checked_add(amount),
                Some(account.treasury - amount),
            )
        };
        let (Some(gold), Some(treasury)) = (gold, treasury) else {
            return Ok(Err("That amount is too large."));
        };
        account.treasury = treasury;
        if deposit && account.missed > 0 && account.treasury >= account.recovery_cost() {
            account.treasury -= account.recovery_cost();
            account.missed = 0;
            account.free_months = 1;
        }
        tx.execute("UPDATE land_estates SET treasury=?2, missed=?3, free_months=?4 WHERE owner_id=?1 AND grade=1",
            params![character.character_id, account.treasury, account.missed, account.free_months])?;
        character.gold = gold;
        Self::write_character_states(&tx, std::slice::from_ref(&character))?;
        Self::replace_inventories(&tx, [(character.character_id, inventory)])?;
        tx.commit()?;
        Ok(Ok((gold, account)))
    }

    pub fn collect_land_taxes(&self, month: i64, online: &[i64]) -> Result<(), AuthError> {
        let online: std::collections::HashSet<_> = online.iter().copied().collect();
        let mut conn = self.open_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO land_tax_periods SELECT id, ?1 FROM land_estates",
            [month],
        )?;
        let mut stmt = tx.prepare(
            "SELECT e.id, e.owner_id, e.treasury, e.missed, e.free_months, t.month,
                    COALESCE(c.last_seen_at, e.created_at), COUNT(p.estate_id)
             FROM land_estates e JOIN land_tax_periods t ON t.estate_id=e.id
             JOIN characters c ON c.id=e.owner_id JOIN land_plots p ON p.estate_id=e.id
             WHERE t.month < ?1 GROUP BY e.id",
        )?;
        let rows = stmt
            .query_map([month], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, u32>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        let month_seconds = (crate::game_state::time::REAL_DAY_DURATION_SECONDS
            * onlinerpg_shared::moon::GAME_DAYS_PER_MONTH as f64)
            as i64;
        for (id, owner, mut treasury, mut missed, mut free, last, seen, plots) in rows {
            let tax = plots * LAND_TAX_PER_PLOT;
            for period in last + 1..=month {
                let inactive = !online.contains(&owner)
                    && super::unix_now() - (month - period) * month_seconds - seen
                        > 8 * month_seconds;
                if free > 0 {
                    free -= 1;
                } else if !inactive && missed == 0 && treasury >= tax {
                    treasury -= tax;
                } else {
                    missed = missed.saturating_add(1);
                }
            }
            tx.execute(
                "UPDATE land_estates SET treasury=?2, missed=?3, free_months=?4 WHERE id=?1",
                params![id, treasury, missed, free],
            )?;
            tx.execute(
                "UPDATE land_tax_periods SET month=?2 WHERE estate_id=?1",
                params![id, month],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

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
            CREATE INDEX IF NOT EXISTS idx_land_plots_estate ON land_plots(estate_id);
            CREATE TABLE IF NOT EXISTS land_tax_periods (
                estate_id INTEGER PRIMARY KEY REFERENCES land_estates(id) ON DELETE CASCADE,
                month INTEGER NOT NULL
            );",
        )
    }

    pub fn claim_homestead(
        &self,
        character: &CharacterSaveData,
        plot: (i32, i32, u8),
        inventory: &[ItemRow],
        month: i64,
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
            "INSERT OR IGNORE INTO land_tax_periods VALUES (?1, ?2)",
            params![estate_id, month],
        )?;
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

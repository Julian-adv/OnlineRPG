use super::{AuthError, AuthService, CharacterSaveData, ItemRow};
use onlinerpg_shared::landscaping::{DEFAULT_PALETTE, PALETTE_ITEMS};
use rusqlite::{params, Connection, TransactionBehavior};

impl AuthService {
    pub(super) fn ensure_landscaping_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS character_landscaping_palettes (
            character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
            palette_slot INTEGER NOT NULL CHECK (palette_slot BETWEEN 0 AND 15),
            PRIMARY KEY (character_id, palette_slot)
        );",
        )
    }

    pub fn landscaping_palette(&self, character_id: i64) -> Result<Vec<u8>, AuthError> {
        let conn = self.open_connection()?;
        let mut query = conn.prepare(
            "SELECT palette_slot FROM character_landscaping_palettes WHERE character_id=?1",
        )?;
        let mut palette = DEFAULT_PALETTE.to_vec();
        palette.extend(
            query
                .query_map([character_id], |row| row.get::<_, u8>(0))?
                .collect::<Result<Vec<_>, _>>()?,
        );
        palette.sort_unstable();
        palette.dedup();
        Ok(palette)
    }

    pub fn unlock_landscaping_palette(
        &self,
        character: &CharacterSaveData,
        inventory: &[ItemRow],
        slot: u8,
    ) -> Result<bool, AuthError> {
        if !PALETTE_ITEMS.iter().any(|(palette, _)| *palette == slot) {
            return Ok(false);
        }
        let mut conn = self.open_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = tx.execute("INSERT OR IGNORE INTO character_landscaping_palettes (character_id, palette_slot) VALUES (?1,?2)",
            params![character.character_id, slot])?;
        if inserted == 0 {
            return Ok(false);
        }
        Self::write_character_states(&tx, std::slice::from_ref(character))?;
        Self::replace_inventories(&tx, [(character.character_id, inventory)])?;
        tx.commit()?;
        Ok(true)
    }
}

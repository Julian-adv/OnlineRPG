use crate::types::{CharacterAttributes, GameDateTime};
use crate::world_config::world_config;
use onlinerpg_shared::{CharacterClass, Gender};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;

/// New characters start with no gold: anything redeemable granted at creation
/// would let abusers mint wealth by recycling characters (see doc/ECONOMY.md).
/// Starter gear instead uses item defs without a basePrice, which merchants
/// refuse to buy. (item_def_id, quantity, equip_slot)
const STARTER_ITEMS: &[(&str, u32, Option<&str>)] = &[
    ("worn_iron_sword", 1, Some("main_hand")),
    ("worn_torch", 1, None),
];

/// Class additions to the starter kit, under the same no-basePrice rule.
fn class_starter_items(
    class: &CharacterClass,
) -> &'static [(&'static str, u32, Option<&'static str>)] {
    match class {
        CharacterClass::Bard => &[("worn_mandolin", 1, None)],
        _ => &[],
    }
}

/// Item defs renamed after release, applied to stored inventories at startup.
/// (old_id, new_id) — new_id must exist in items.csv.
const RENAMED_ITEM_IDS: &[(&str, &str)] = &[
    ("leather_cap", "leather_helmet"),
    ("iron_chestplate", "breastplate"),
];

/// Reserved account-name prefix for headless NPC/bot accounts.
pub const NPC_ACCOUNT_PREFIX: &str = "npc_";

const MAX_NAME_CHARS: usize = 32;

/// Names end up in logs, chat and the UI, so allowlist characters instead of
/// chasing Unicode lookalikes/invisibles: ASCII alphanumeric, underscore,
/// Hangul (syllables + modern jamo), kana (+ ー), CJK unified ideographs.
fn valid_name_char(c: char) -> bool {
    matches!(c,
        'a'..='z' | 'A'..='Z' | '0'..='9' | '_'
        | '가'..='힣' | 'ㄱ'..='ㅣ'
        | 'ぁ'..='ゖ' | 'ァ'..='ヺ' | 'ー'
        | '一'..='\u{9fff}')
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= MAX_NAME_CHARS && name.chars().all(valid_name_char)
}

/// Character names must also start with a letter; digit/underscore-leading
/// names created before this rule are grandfathered.
fn valid_character_name(name: &str) -> bool {
    valid_name(name)
        && name
            .chars()
            .next()
            .is_some_and(|c| !matches!(c, '0'..='9' | '_'))
}

/// One persisted inventory row: a bag stack (`equip_slot: None`) or an
/// equipped item.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRow {
    pub item_def_id: String,
    pub quantity: u32,
    pub equip_slot: Option<String>,
    pub enchant: i32,
}

/// One trained skill as stored in `character_skills`. The skill id is kept as
/// its wire string (`SkillId::as_str`) so rows written by a newer server
/// survive a rollback: unknown ids load as rows, get skipped at the
/// `Skills` conversion, and are preserved on the next save.
#[derive(Debug, Clone)]
pub struct SkillRow {
    pub skill_id: String,
    pub level: u32,
    pub xp: u64,
}

/// A ban in force on an account. `until_unix` is `None` for a permanent ban.
#[derive(Debug, Clone)]
pub struct AccountBan {
    pub reason: Option<String>,
    pub until_unix: Option<i64>,
}

impl AccountBan {
    pub fn message(&self) -> String {
        ban_message(self.reason.as_deref(), self.until_unix)
    }
}

pub const DEFAULT_BAN_REASON: &str = "Banned by an operator";

/// Client-facing text, so a kicked player learns why and for how long.
pub fn ban_message(reason: Option<&str>, until_unix: Option<i64>) -> String {
    let reason = reason.unwrap_or(DEFAULT_BAN_REASON);
    match until_unix {
        None => reason.to_string(),
        Some(until) => {
            let minutes = ((until - unix_now()).max(0) as u64).div_ceil(60);
            format!("{reason} ({minutes} minute(s) remaining)")
        }
    }
}

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct AuthService {
    pool: r2d2::Pool<SqliteConnectionManager>,
}

#[derive(Debug, Clone)]
pub struct CharacterRecord {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub level: u32,
    pub xp: u64,
    pub max_hp: u32,
    pub attributes: CharacterAttributes,
    pub class: CharacterClass,
    pub gender: Gender,
    pub last_x: f32,
    pub last_y: f32,
    pub last_z: f32,
    pub last_rotation: f32,
    pub health: Option<u32>,
    pub floor_level: i8,
    pub gold: i64,
    /// Nonzero unlocks admin for ADMIN_EMAILS-allowlisted accounts (tiers reserved).
    pub admin_role: i64,
    pub satiation: u32,
}

pub struct CharacterSaveData {
    pub character_id: i64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
    pub xp: u64,
    pub level: u32,
    pub max_hp: u32,
    pub health: u32,
    pub floor_level: i8,
    pub gold: i64,
    pub satiation: u32,
}

/// One row of the player-trade ledger. `*_items` are JSON arrays of
/// `{def, qty, ench}` — never `instance_id`, which is minted per session and
/// means nothing once the session ends.
pub struct TradeLedgerEntry {
    pub a_character_id: i64,
    pub b_character_id: i64,
    pub a_gold_before: i64,
    pub a_gold_after: i64,
    pub b_gold_before: i64,
    pub b_gold_after: i64,
    pub a_items: String,
    pub b_items: String,
}

/// Column list shared between queries that return full CharacterRecord rows.
const CHARACTER_COLUMNS: &str = "id, character_name, created_at, level, xp, max_hp, attr_str, attr_dex, attr_con, attr_int, attr_wis, attr_cha, attr_guard, class, last_x, last_y, last_z, last_rotation, health, floor_level, gender, gold, admin_role, satiation";

fn character_record_from_row(row: &rusqlite::Row) -> rusqlite::Result<CharacterRecord> {
    Ok(CharacterRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
        level: row.get(3)?,
        xp: row.get::<_, i64>(4)? as u64,
        max_hp: row.get(5)?,
        attributes: CharacterAttributes {
            r#str: row.get(6)?,
            dex: row.get(7)?,
            con: row.get(8)?,
            int: row.get(9)?,
            wis: row.get(10)?,
            cha: row.get(11)?,
            guard: row.get(12)?,
        },
        class: {
            let class_str: String = row.get(13)?;
            class_str.parse::<CharacterClass>().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    13,
                    rusqlite::types::Type::Text,
                    format!("unknown character class: {class_str}").into(),
                )
            })?
        },
        last_x: row.get::<_, f64>(14).unwrap_or(0.0) as f32,
        last_y: row.get::<_, f64>(15).unwrap_or(0.0) as f32,
        last_z: row.get::<_, f64>(16).unwrap_or(0.0) as f32,
        last_rotation: row.get::<_, f64>(17).unwrap_or(0.0) as f32,
        health: row
            .get::<_, Option<i64>>(18)
            .ok()
            .flatten()
            .map(|v| v as u32),
        floor_level: row.get::<_, i64>(19).unwrap_or(0) as i8,
        gender: match row
            .get::<_, String>(20)
            .unwrap_or_else(|_| "male".to_string())
            .as_str()
        {
            "female" => Gender::Female,
            _ => Gender::Male,
        },
        gold: row.get::<_, i64>(21).unwrap_or(0),
        admin_role: row.get::<_, i64>(22).unwrap_or(0),
        satiation: row
            .get::<_, i64>(23)
            .unwrap_or(i64::from(onlinerpg_shared::hunger::SATIATION_START))
            .clamp(0, i64::from(onlinerpg_shared::hunger::SATIATION_MAX)) as u32,
    })
}

#[derive(Debug)]
pub enum AuthError {
    InvalidInput(&'static str),
    AccountNotFound,
    InvalidCharacterName,
    CharacterLimitReached,
    CharacterNameAlreadyExists,
    CharacterNotFound,
    Database(String),
}

impl AuthError {
    pub fn client_message(&self) -> &'static str {
        match self {
            AuthError::InvalidInput(message) => message,
            AuthError::AccountNotFound => "Account not found",
            AuthError::InvalidCharacterName => {
                "Character name must start with a letter and contain only letters, digits, or _"
            }
            AuthError::CharacterLimitReached => {
                "A maximum of 3 characters can be created per account"
            }
            AuthError::CharacterNameAlreadyExists => "Character name already exists",
            AuthError::CharacterNotFound => "Character not found",
            AuthError::Database(_) => "Server auth database error",
        }
    }
}

impl Display for AuthError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Database(message) => write!(f, "Database error: {message}"),
            other => write!(f, "{}", other.client_message()),
        }
    }
}

impl std::error::Error for AuthError {}

impl From<rusqlite::Error> for AuthError {
    fn from(e: rusqlite::Error) -> Self {
        AuthError::Database(e.to_string())
    }
}

impl AuthService {
    fn write_character_states(
        conn: &Connection,
        data: &[CharacterSaveData],
    ) -> Result<(), rusqlite::Error> {
        let mut stmt = conn.prepare(
            "UPDATE characters SET last_x = ?1, last_y = ?2, last_z = ?3, last_rotation = ?4, \
             xp = ?5, level = ?6, max_hp = ?7, health = ?8, floor_level = ?9, gold = ?10, \
             satiation = ?11 WHERE id = ?12",
        )?;
        for d in data {
            stmt.execute(params![
                f64::from(d.x),
                f64::from(d.y),
                f64::from(d.z),
                f64::from(d.rotation),
                d.xp as i64,
                i64::from(d.level),
                i64::from(d.max_hp),
                i64::from(d.health),
                i64::from(d.floor_level),
                d.gold,
                i64::from(d.satiation),
                d.character_id,
            ])?;
        }
        Ok(())
    }

    fn write_world_time(conn: &Connection, datetime: &GameDateTime) -> Result<(), rusqlite::Error> {
        conn.execute(
            "INSERT INTO world_time (id, year, month, day, hour, minute, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, strftime('%s', 'now'))
             ON CONFLICT(id) DO UPDATE SET
                year = excluded.year,
                month = excluded.month,
                day = excluded.day,
                hour = excluded.hour,
                minute = excluded.minute,
                updated_at = excluded.updated_at",
            params![
                i64::from(datetime.year),
                i64::from(datetime.month),
                i64::from(datetime.day),
                i64::from(datetime.hour),
                i64::from(datetime.minute),
            ],
        )?;
        Ok(())
    }

    fn replace_inventories<'a>(
        conn: &Connection,
        inventories: impl IntoIterator<Item = (i64, &'a [ItemRow])>,
    ) -> Result<(), rusqlite::Error> {
        let mut delete = conn.prepare("DELETE FROM character_items WHERE character_id = ?1")?;
        let mut insert = conn.prepare(
            "INSERT INTO character_items (character_id, item_def_id, quantity, equip_slot, enchant) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;

        for (character_id, items) in inventories {
            delete.execute(params![character_id])?;
            for item in items {
                insert.execute(params![
                    character_id,
                    item.item_def_id,
                    item.quantity,
                    item.equip_slot,
                    item.enchant
                ])?;
            }
        }
        Ok(())
    }

    /// Upsert, not delete+insert like inventories: skills are only ever added
    /// or advanced, and an upsert leaves rows a newer server wrote (unknown
    /// skill ids) untouched across a rollback.
    fn upsert_skills<'a>(
        conn: &Connection,
        skills: impl IntoIterator<Item = (i64, &'a [SkillRow])>,
    ) -> Result<(), rusqlite::Error> {
        let mut upsert = conn.prepare(
            "INSERT INTO character_skills (character_id, skill_id, level, xp) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(character_id, skill_id) DO UPDATE SET
                level = excluded.level,
                xp = excluded.xp",
        )?;

        for (character_id, rows) in skills {
            for row in rows {
                upsert.execute(params![
                    character_id,
                    row.skill_id,
                    row.level,
                    row.xp as i64
                ])?;
            }
        }
        Ok(())
    }

    pub fn new(db_path: PathBuf) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let manager = SqliteConnectionManager::file(&db_path)
            .with_init(|conn| conn.execute_batch("PRAGMA foreign_keys = ON"));

        let pool = r2d2::Pool::builder().build(manager)?;

        let conn = pool.get()?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS accounts (
                player_name TEXT PRIMARY KEY,
                google_sub TEXT,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;
        Self::ensure_accounts_columns(&conn)?;
        Self::ensure_characters_schema(&conn)?;
        Self::migrate_item_definition_ids(&conn)?;
        Self::ensure_blocks_schema(&conn)?;
        Self::ensure_friends_schema(&conn)?;
        Self::ensure_bans_schema(&conn)?;
        Self::ensure_character_skills_schema(&conn)?;
        Self::ensure_world_time_schema(&conn)?;
        Self::ensure_dungeon_chest_schema(&conn)?;
        Self::ensure_dungeon_discovery_schema(&conn)?;
        Self::ensure_trade_ledger_schema(&conn)?;

        Ok(Self { pool })
    }

    /// Migrate pre-Google-auth databases: the FNV password hashes are dropped
    /// (worthless as credentials) and accounts become reachable only via
    /// `google_sub` (browser) or the NPC token path.
    fn ensure_accounts_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
        let columns = Self::table_columns(conn, "accounts")?;
        if columns.contains("password_hash") {
            conn.execute("ALTER TABLE accounts DROP COLUMN password_hash", [])?;
        }
        if !columns.contains("google_sub") {
            conn.execute("ALTER TABLE accounts ADD COLUMN google_sub TEXT", [])?;
        }
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_accounts_google_sub
             ON accounts(google_sub) WHERE google_sub IS NOT NULL",
            [],
        )?;
        Ok(())
    }

    fn ensure_characters_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS characters (
                id INTEGER PRIMARY KEY,
                account_name TEXT NOT NULL,
                character_name TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                level INTEGER NOT NULL DEFAULT 1,
                max_hp INTEGER NOT NULL DEFAULT 16,
                attr_str INTEGER NOT NULL DEFAULT 12,
                attr_dex INTEGER NOT NULL DEFAULT 12,
                attr_con INTEGER NOT NULL DEFAULT 12,
                attr_int INTEGER NOT NULL DEFAULT 12,
                attr_wis INTEGER NOT NULL DEFAULT 12,
                attr_cha INTEGER NOT NULL DEFAULT 12,
                attr_guard INTEGER NOT NULL DEFAULT 10,
                FOREIGN KEY (account_name) REFERENCES accounts(player_name) ON DELETE CASCADE
            )",
            [],
        )?;
        Self::ensure_character_attribute_columns(conn)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_characters_account_name ON characters(account_name)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS character_items (
                id INTEGER PRIMARY KEY,
                character_id INTEGER NOT NULL,
                item_def_id TEXT NOT NULL,
                quantity INTEGER NOT NULL DEFAULT 1,
                equip_slot TEXT,
                enchant INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (character_id) REFERENCES characters(id) ON DELETE CASCADE
            )",
            [],
        )?;
        Self::ensure_character_item_columns(conn)?;

        // Every inventory read and every save's DELETE filters on character_id;
        // without this SQLite full-scans the table once per character.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_character_items_character_id \
             ON character_items(character_id)",
            [],
        )?;

        Ok(())
    }

    /// Friendships, one row per direction. Ids rather than names (unlike
    /// `character_blocks`): deleting a character must take its friendships
    /// with it, which both cascades give for free, and a name would leave a
    /// permanently-offline ghost on every friend's list.
    fn ensure_friends_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS character_friends (
                character_id INTEGER NOT NULL,
                friend_id INTEGER NOT NULL,
                PRIMARY KEY (character_id, friend_id),
                FOREIGN KEY (character_id) REFERENCES characters(id) ON DELETE CASCADE,
                FOREIGN KEY (friend_id) REFERENCES characters(id) ON DELETE CASCADE
            )",
            [],
        )?;
        // The reverse-direction cascade needs it, and so does nothing else:
        // every read is by `character_id`, which the primary key covers.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_character_friends_friend_id \
             ON character_friends(friend_id)",
            [],
        )?;
        Ok(())
    }

    /// `/block` lists: character → set of character names it never wants to
    /// hear. Names rather than character ids, so a block written while the
    /// target is offline needs no id lookup and survives the target's
    /// character deletion (a recreated abuser keeps the same name).
    fn ensure_blocks_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS character_blocks (
                character_id INTEGER NOT NULL,
                blocked_name TEXT NOT NULL,
                PRIMARY KEY (character_id, blocked_name),
                FOREIGN KEY (character_id) REFERENCES characters(id) ON DELETE CASCADE
            )",
            [],
        )?;
        // Names are unique ignoring ASCII case (the other allowed scripts
        // have no case); the index also serves the COLLATE NOCASE lookups.
        // A DB with case-colliding names fails here on purpose: rename the
        // offending characters before starting.
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_characters_name_unique_nocase \
             ON characters(character_name COLLATE NOCASE)",
            [],
        )?;
        conn.execute("DROP INDEX IF EXISTS idx_characters_name_nocase", [])?;
        Ok(())
    }

    /// Column names currently on `table`, for post-release ALTER migrations.
    fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>, rusqlite::Error> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<HashSet<_>, _>>()?;
        Ok(columns)
    }

    /// Columns added to character_items after release; mirrors
    /// `ensure_character_attribute_columns` for the characters table.
    fn ensure_character_item_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
        if !Self::table_columns(conn, "character_items")?.contains("enchant") {
            conn.execute(
                "ALTER TABLE character_items ADD COLUMN enchant INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }

        Ok(())
    }

    fn migrate_item_definition_ids(conn: &Connection) -> Result<(), rusqlite::Error> {
        let mut stmt =
            conn.prepare("UPDATE character_items SET item_def_id = ?1 WHERE item_def_id = ?2")?;
        let mut migrated = 0;
        for &(old, new) in RENAMED_ITEM_IDS {
            migrated += stmt.execute(params![new, old])?;
        }
        if migrated > 0 {
            tracing::info!(migrated, "Migrated legacy item definition ids");
        }
        Ok(())
    }

    /// When each character last opened a dungeon's treasure chest, in world
    /// clock seconds. Stored against the game clock (also persisted, see
    /// `world_time`) rather than wall time because the chest refills at
    /// nightfall; keeping the raw timestamp instead of a derived night index
    /// leaves the refill rule free to change later.
    fn ensure_dungeon_chest_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS character_dungeon_chests (
                character_id INTEGER NOT NULL,
                entrance_id TEXT NOT NULL,
                opened_game_seconds INTEGER NOT NULL,
                PRIMARY KEY (character_id, entrance_id),
                FOREIGN KEY (character_id) REFERENCES characters(id) ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    /// Bans key on the account, not the character: a banned player can delete
    /// and recreate characters, but the Google subject stays put. `until_unix`
    /// is NULL for a permanent ban and an epoch second for a timed one —
    /// wall-clock, because a monotonic `Instant` cannot survive a restart.
    fn ensure_bans_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS account_bans (
                account_name TEXT PRIMARY KEY,
                reason TEXT,
                until_unix INTEGER,
                banned_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (account_name) REFERENCES accounts(player_name) ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    /// Dungeon entrances each character has discovered (world-map markers).
    /// Row presence is the whole fact — losing one only means rediscovering
    /// by walking near the entrance again.
    fn ensure_dungeon_discovery_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS character_dungeon_discoveries (
                character_id INTEGER NOT NULL,
                entrance_id TEXT NOT NULL,
                PRIMARY KEY (character_id, entrance_id),
                FOREIGN KEY (character_id) REFERENCES characters(id) ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    fn ensure_character_skills_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS character_skills (
                character_id INTEGER NOT NULL,
                skill_id TEXT NOT NULL,
                level INTEGER NOT NULL DEFAULT 0,
                xp INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (character_id, skill_id),
                FOREIGN KEY (character_id) REFERENCES characters(id) ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    /// Player-trade ledger (doc/TRADE.md). Written in the same transaction as
    /// the trade itself, so it can never disagree with what happened. Gold
    /// before and after both sides is what makes coin appearing from nowhere
    /// detectable. No foreign key: a deleted character must not erase the
    /// record of what they traded away.
    fn ensure_trade_ledger_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS player_trades (
                id INTEGER PRIMARY KEY,
                traded_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                a_character_id INTEGER NOT NULL,
                b_character_id INTEGER NOT NULL,
                a_gold_before INTEGER NOT NULL,
                a_gold_after INTEGER NOT NULL,
                b_gold_before INTEGER NOT NULL,
                b_gold_after INTEGER NOT NULL,
                a_items TEXT NOT NULL,
                b_items TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_player_trades_a ON player_trades(a_character_id)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_player_trades_b ON player_trades(b_character_id)",
            [],
        )?;
        Ok(())
    }

    fn ensure_world_time_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS world_time (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                day INTEGER NOT NULL,
                hour INTEGER NOT NULL,
                minute INTEGER NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;
        Ok(())
    }

    fn ensure_character_attribute_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
        let existing_columns = Self::table_columns(conn, "characters")?;

        let spawn = &world_config().spawn_position;
        let expected_columns: Vec<(&str, String)> = vec![
            ("level", "INTEGER NOT NULL DEFAULT 1".into()),
            ("xp", "INTEGER NOT NULL DEFAULT 0".into()),
            ("max_hp", "INTEGER NOT NULL DEFAULT 16".into()),
            ("attr_str", "INTEGER NOT NULL DEFAULT 12".into()),
            ("attr_dex", "INTEGER NOT NULL DEFAULT 12".into()),
            ("attr_con", "INTEGER NOT NULL DEFAULT 12".into()),
            ("attr_int", "INTEGER NOT NULL DEFAULT 12".into()),
            ("attr_wis", "INTEGER NOT NULL DEFAULT 12".into()),
            ("attr_cha", "INTEGER NOT NULL DEFAULT 12".into()),
            ("attr_guard", "INTEGER NOT NULL DEFAULT 10".into()),
            ("class", "TEXT NOT NULL DEFAULT 'knight'".into()),
            ("last_x", format!("REAL NOT NULL DEFAULT {}", spawn.x)),
            ("last_y", format!("REAL NOT NULL DEFAULT {}", spawn.y)),
            ("last_z", format!("REAL NOT NULL DEFAULT {}", spawn.z)),
            (
                "last_rotation",
                format!("REAL NOT NULL DEFAULT {}", spawn.rotation),
            ),
            ("health", "INTEGER".into()),
            ("floor_level", "INTEGER NOT NULL DEFAULT 0".into()),
            ("gender", "TEXT NOT NULL DEFAULT 'male'".into()),
            ("gold", "INTEGER NOT NULL DEFAULT 0".into()),
            ("admin_role", "INTEGER NOT NULL DEFAULT 0".into()),
            (
                "satiation",
                format!(
                    "INTEGER NOT NULL DEFAULT {}",
                    onlinerpg_shared::hunger::SATIATION_START
                ),
            ),
        ];

        for (column_name, column_def) in &expected_columns {
            if !existing_columns.contains(*column_name) {
                let sql = format!(
                    "ALTER TABLE characters ADD COLUMN {} {}",
                    column_name, column_def
                );
                conn.execute(sql.as_str(), [])?;
            }
        }

        Ok(())
    }

    fn open_connection(
        &self,
    ) -> Result<r2d2::PooledConnection<SqliteConnectionManager>, AuthError> {
        self.pool
            .get()
            .map_err(|e| AuthError::Database(e.to_string()))
    }

    /// Log in with a verified Google subject id, creating the account on
    /// first login. Returns the account's player_name. Account names are
    /// random on purpose — deriving them from token claims (email/name)
    /// would persist personal data.
    pub fn login_google(&self, google_sub: &str) -> Result<String, AuthError> {
        let google_sub = google_sub.trim();
        if google_sub.is_empty() {
            return Err(AuthError::InvalidInput("Google subject id is required"));
        }

        let conn = self.open_connection()?;

        for _ in 0..100 {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT player_name FROM accounts WHERE google_sub = ?1",
                    params![google_sub],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(name) = existing {
                return Ok(name);
            }

            let candidate = format!("player_{}", &uuid::Uuid::new_v4().simple().to_string()[..6]);
            match conn.execute(
                "INSERT INTO accounts (player_name, google_sub) VALUES (?1, ?2)",
                params![candidate, google_sub],
            ) {
                Ok(_) => return Ok(candidate),
                // Name taken (or lost a same-sub race): retry with a fresh name.
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    continue
                }
                Err(e) => return Err(e.into()),
            }
        }

        Err(AuthError::Database(
            "could not allocate a unique account name".to_string(),
        ))
    }

    /// Log in a headless NPC account (token already checked by the caller),
    /// creating it on first use. Returns the canonical (trimmed) name.
    ///
    /// NPC accounts live in a reserved `npc_` namespace: player accounts are
    /// named `player_*` (Google) or predate this scheme (legacy), so requiring
    /// the prefix stops the shared NPC token from ever binding to a human's
    /// account, even on a config typo.
    pub fn login_npc(&self, account_name: &str) -> Result<String, AuthError> {
        let account_name = account_name.trim();
        if account_name.is_empty() {
            return Err(AuthError::InvalidInput("Account name is required"));
        }
        if !account_name.starts_with(NPC_ACCOUNT_PREFIX) {
            return Err(AuthError::InvalidInput(
                "NPC account names must start with 'npc_'",
            ));
        }
        if !valid_name(account_name) {
            return Err(AuthError::InvalidInput(
                "Account name is too long or contains invalid characters",
            ));
        }

        let conn = self.open_connection()?;
        let existing_sub: Option<Option<String>> = conn
            .query_row(
                "SELECT google_sub FROM accounts WHERE player_name = ?1",
                params![account_name],
                |row| row.get(0),
            )
            .optional()?;

        match existing_sub {
            Some(None) => Ok(account_name.to_string()),
            Some(Some(_)) => Err(AuthError::InvalidInput(
                "Account name belongs to a player account",
            )),
            None => {
                conn.execute(
                    "INSERT INTO accounts (player_name) VALUES (?1)",
                    params![account_name],
                )?;
                Ok(account_name.to_string())
            }
        }
    }

    pub fn list_characters(&self, account_name: &str) -> Result<Vec<CharacterRecord>, AuthError> {
        let account_name = account_name.trim();
        if account_name.is_empty() {
            return Err(AuthError::InvalidInput("Account name is required"));
        }

        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {}
             FROM characters
             WHERE account_name = ?1
             ORDER BY created_at ASC, id ASC",
            CHARACTER_COLUMNS
        ))?;

        let characters = stmt
            .query_map(params![account_name], character_record_from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(characters)
    }

    /// Id and canonical name of an existing character, matched ignoring
    /// ASCII case (the in-memory `match_name` rule in SQL — SQLite NOCASE is
    /// ASCII-only, like `eq_ignore_ascii_case`).
    pub fn resolve_character_brief(&self, name: &str) -> Result<Option<(i64, String)>, AuthError> {
        let conn = self.open_connection()?;
        let found = conn
            .query_row(
                "SELECT id, character_name FROM characters
                 WHERE character_name = ?1 COLLATE NOCASE",
                params![name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(found)
    }

    /// One character's friends as (id, name, level). The join is what makes
    /// storing ids affordable: offline friends still have a name to show.
    pub fn load_friends(&self, character_id: i64) -> Result<Vec<(i64, String, u32)>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.character_name, c.level
             FROM character_friends f
             JOIN characters c ON c.id = f.friend_id
             WHERE f.character_id = ?1",
        )?;
        let friends = stmt
            .query_map(params![character_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(friends)
    }

    /// Both directions in one transaction, so no crash can leave a one-sided
    /// friendship the callers never expect to see.
    pub fn add_friend(&self, character_id: i64, friend_id: i64) -> Result<(), AuthError> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;
        for (a, b) in [(character_id, friend_id), (friend_id, character_id)] {
            tx.execute(
                "INSERT OR IGNORE INTO character_friends (character_id, friend_id) \
                 VALUES (?1, ?2)",
                params![a, b],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_friend(&self, character_id: i64, friend_id: i64) -> Result<(), AuthError> {
        let conn = self.open_connection()?;
        conn.execute(
            "DELETE FROM character_friends \
             WHERE (character_id = ?1 AND friend_id = ?2) \
                OR (character_id = ?2 AND friend_id = ?1)",
            params![character_id, friend_id],
        )?;
        Ok(())
    }

    pub fn load_blocked_names(&self, character_id: i64) -> Result<Vec<String>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt =
            conn.prepare("SELECT blocked_name FROM character_blocks WHERE character_id = ?1")?;
        let names = stmt
            .query_map(params![character_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names)
    }

    pub fn add_block(&self, character_id: i64, blocked_name: &str) -> Result<(), AuthError> {
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT OR IGNORE INTO character_blocks (character_id, blocked_name) VALUES (?1, ?2)",
            params![character_id, blocked_name],
        )?;
        Ok(())
    }

    pub fn remove_block(&self, character_id: i64, blocked_name: &str) -> Result<(), AuthError> {
        let conn = self.open_connection()?;
        conn.execute(
            "DELETE FROM character_blocks WHERE character_id = ?1 AND blocked_name = ?2",
            params![character_id, blocked_name],
        )?;
        Ok(())
    }

    /// Canonical name and owning account for a character, matched ignoring
    /// ASCII case like the other name lookups. `None` when no such character
    /// exists.
    pub fn account_of_character(
        &self,
        character_name: &str,
    ) -> Result<Option<(String, String)>, AuthError> {
        let conn = self.open_connection()?;
        let found = conn
            .query_row(
                "SELECT character_name, account_name FROM characters
                 WHERE character_name = ?1 COLLATE NOCASE",
                params![character_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(found)
    }

    /// Ban an account, replacing any existing ban so a re-ban can extend or
    /// shorten it. `until_unix` is `None` for a permanent ban.
    pub fn ban_account(
        &self,
        account_name: &str,
        reason: Option<&str>,
        until_unix: Option<i64>,
    ) -> Result<(), AuthError> {
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO account_bans (account_name, reason, until_unix)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(account_name) DO UPDATE SET
                reason = excluded.reason,
                until_unix = excluded.until_unix,
                banned_at = strftime('%s', 'now')",
            params![account_name, reason, until_unix],
        )?;
        Ok(())
    }

    pub fn unban_account(&self, account_name: &str) -> Result<bool, AuthError> {
        let conn = self.open_connection()?;
        let removed = conn.execute(
            "DELETE FROM account_bans WHERE account_name = ?1",
            params![account_name],
        )?;
        Ok(removed > 0)
    }

    /// The ban in force on an account right now, or `None`. An expired row is
    /// deleted on read so the table does not accumulate dead bans.
    pub fn active_ban(&self, account_name: &str) -> Result<Option<AccountBan>, AuthError> {
        let conn = self.open_connection()?;
        let row: Option<(Option<String>, Option<i64>)> = conn
            .query_row(
                "SELECT reason, until_unix FROM account_bans WHERE account_name = ?1",
                params![account_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((reason, until_unix)) = row else {
            return Ok(None);
        };
        if let Some(until) = until_unix {
            if until <= unix_now() {
                // Scoped to the deadline just read: a re-ban landing between
                // the select and this delete carries a different `until_unix`
                // (or NULL) and must survive.
                conn.execute(
                    "DELETE FROM account_bans
                     WHERE account_name = ?1 AND until_unix = ?2",
                    params![account_name, until],
                )?;
                return Ok(None);
            }
        }
        Ok(Some(AccountBan { reason, until_unix }))
    }

    fn dungeon_chest_opens_on(
        conn: &Connection,
        character_id: i64,
    ) -> Result<Vec<(String, i64)>, rusqlite::Error> {
        let mut stmt = conn.prepare(
            "SELECT entrance_id, opened_game_seconds FROM character_dungeon_chests \
             WHERE character_id = ?1",
        )?;
        let opens = stmt
            .query_map(params![character_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(opens)
    }

    fn dungeon_discoveries_on(
        conn: &Connection,
        character_id: i64,
    ) -> Result<Vec<String>, rusqlite::Error> {
        let mut stmt = conn.prepare(
            "SELECT entrance_id FROM character_dungeon_discoveries WHERE character_id = ?1",
        )?;
        let ids = stmt
            .query_map(params![character_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Chest opens and discovered entrances together, as ((entrance id,
    /// world clock seconds) pairs, entrance ids), sharing one connection.
    /// Read once at login into `GameState`.
    #[allow(clippy::type_complexity)]
    pub fn load_dungeon_history(
        &self,
        character_id: i64,
    ) -> Result<(Vec<(String, i64)>, Vec<String>), AuthError> {
        let conn = self.open_connection()?;
        Ok((
            Self::dungeon_chest_opens_on(&conn, character_id)?,
            Self::dungeon_discoveries_on(&conn, character_id)?,
        ))
    }

    pub fn record_dungeon_chest_open(
        &self,
        character_id: i64,
        entrance_id: &str,
        opened_game_seconds: i64,
    ) -> Result<(), AuthError> {
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO character_dungeon_chests \
                 (character_id, entrance_id, opened_game_seconds) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(character_id, entrance_id) \
             DO UPDATE SET opened_game_seconds = excluded.opened_game_seconds",
            params![character_id, entrance_id, opened_game_seconds],
        )?;
        Ok(())
    }

    fn insert_dungeon_discoveries(
        conn: &Connection,
        rows: &[(i64, String)],
    ) -> Result<(), rusqlite::Error> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut insert = conn.prepare(
            "INSERT OR IGNORE INTO character_dungeon_discoveries \
                 (character_id, entrance_id) VALUES (?1, ?2)",
        )?;
        for (character_id, entrance_id) in rows {
            insert.execute(params![character_id, entrance_id])?;
        }
        Ok(())
    }

    pub fn create_character(
        &self,
        account_name: &str,
        character_name: &str,
        attributes: &CharacterAttributes,
        max_hp: u32,
        class: CharacterClass,
        gender: Gender,
    ) -> Result<CharacterRecord, AuthError> {
        let account_name = account_name.trim();
        let character_name = character_name.trim();

        if account_name.is_empty() {
            return Err(AuthError::InvalidInput("Account name is required"));
        }

        if !valid_character_name(character_name) {
            return Err(AuthError::InvalidCharacterName);
        }

        let conn = self.open_connection()?;

        let account_exists: Option<String> = conn
            .query_row(
                "SELECT player_name FROM accounts WHERE player_name = ?1",
                params![account_name],
                |row| row.get(0),
            )
            .optional()?;
        if account_exists.is_none() {
            return Err(AuthError::AccountNotFound);
        }

        let character_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM characters WHERE account_name = ?1",
            params![account_name],
            |row| row.get(0),
        )?;
        if character_count >= 3 {
            return Err(AuthError::CharacterLimitReached);
        }

        let existing_character_name: Option<String> = conn
            .query_row(
                "SELECT character_name FROM characters \
                 WHERE character_name = ?1 COLLATE NOCASE",
                params![character_name],
                |row| row.get(0),
            )
            .optional()?;
        if existing_character_name.is_some() {
            return Err(AuthError::CharacterNameAlreadyExists);
        }

        let gender_str = match gender {
            Gender::Male => "male",
            Gender::Female => "female",
        };

        conn.execute(
            "INSERT INTO characters (
                account_name,
                character_name,
                level,
                max_hp,
                attr_str,
                attr_dex,
                attr_con,
                attr_int,
                attr_wis,
                attr_cha,
                attr_guard,
                class,
                gender,
                last_x,
                last_y,
                last_z,
                last_rotation,
                gold
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, 0)",
            params![
                account_name,
                character_name,
                1_i64,
                i64::from(max_hp),
                i64::from(attributes.r#str),
                i64::from(attributes.dex),
                i64::from(attributes.con),
                i64::from(attributes.int),
                i64::from(attributes.wis),
                i64::from(attributes.cha),
                i64::from(attributes.guard),
                class.as_str(),
                gender_str,
                f64::from(world_config().spawn_position.x),
                f64::from(world_config().spawn_position.y),
                f64::from(world_config().spawn_position.z),
                f64::from(world_config().spawn_position.rotation),
            ],
        )?;

        let id = conn.last_insert_rowid();

        // A registry NPC with an issued loadout skips the starter kit —
        // otherwise the worn starter sword would occupy main_hand. The gear
        // itself is granted and worn by `seed_npc_loadout` on every join.
        let issued = account_name.starts_with(NPC_ACCOUNT_PREFIX)
            && crate::npc_defs::npc_defs()
                .get_by_npc_name(character_name)
                .is_some_and(|def| !def.loadout.is_empty());
        if !issued {
            let mut stmt = conn.prepare(
                "INSERT INTO character_items (character_id, item_def_id, quantity, equip_slot) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (item_def_id, quantity, equip_slot) in
                STARTER_ITEMS.iter().chain(class_starter_items(&class))
            {
                stmt.execute(params![id, item_def_id, quantity, equip_slot])?;
            }
        }
        let created_at: i64 = conn.query_row(
            "SELECT created_at FROM characters WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;

        Ok(CharacterRecord {
            id,
            name: character_name.to_string(),
            created_at,
            level: 1,
            xp: 0,
            max_hp,
            attributes: attributes.clone(),
            class,
            gender,
            last_x: world_config().spawn_position.x,
            last_y: world_config().spawn_position.y,
            last_z: world_config().spawn_position.z,
            last_rotation: world_config().spawn_position.rotation,
            health: None,
            floor_level: 0,
            gold: 0,
            admin_role: 0,
            satiation: onlinerpg_shared::hunger::SATIATION_START,
        })
    }

    pub fn delete_character(&self, account_name: &str, character_id: i64) -> Result<(), AuthError> {
        let account_name = account_name.trim();
        if account_name.is_empty() {
            return Err(AuthError::InvalidInput("Account name is required"));
        }
        if character_id <= 0 {
            return Err(AuthError::CharacterNotFound);
        }

        let conn = self.open_connection()?;
        let rows_affected = conn.execute(
            "DELETE FROM characters WHERE id = ?1 AND account_name = ?2",
            params![character_id, account_name],
        )?;

        if rows_affected == 0 {
            return Err(AuthError::CharacterNotFound);
        }

        Ok(())
    }

    pub fn get_character_for_account(
        &self,
        account_name: &str,
        character_id: i64,
    ) -> Result<CharacterRecord, AuthError> {
        let account_name = account_name.trim();
        if account_name.is_empty() {
            return Err(AuthError::InvalidInput("Account name is required"));
        }
        if character_id <= 0 {
            return Err(AuthError::CharacterNotFound);
        }

        let conn = self.open_connection()?;
        let character = conn
            .query_row(
                &format!(
                    "SELECT {}
                     FROM characters
                     WHERE id = ?1 AND account_name = ?2",
                    CHARACTER_COLUMNS
                ),
                params![character_id, account_name],
                character_record_from_row,
            )
            .optional()?;

        character.ok_or(AuthError::CharacterNotFound)
    }

    /// The one write path for game state: the periodic flush, a single player's
    /// logout and the shutdown snapshot all land here. Everything goes in one
    /// transaction, so a save costs one commit no matter how much it covers.
    pub fn save_batch(
        &self,
        characters: &[CharacterSaveData],
        inventories: &[(i64, Vec<ItemRow>)],
        skills: &[(i64, Vec<SkillRow>)],
        discoveries: &[(i64, String)],
        world_time: Option<&GameDateTime>,
    ) -> Result<(), AuthError> {
        if characters.is_empty()
            && inventories.is_empty()
            && skills.is_empty()
            && discoveries.is_empty()
            && world_time.is_none()
        {
            return Ok(());
        }
        let conn = self.open_connection()?;
        let tx = conn.unchecked_transaction()?;
        Self::write_character_states(&tx, characters)?;
        Self::replace_inventories(
            &tx,
            inventories
                .iter()
                .map(|(id, items)| (*id, items.as_slice())),
        )?;
        Self::upsert_skills(&tx, skills.iter().map(|(id, rows)| (*id, rows.as_slice())))?;
        Self::insert_dungeon_discoveries(&tx, discoveries)?;
        if let Some(datetime) = world_time {
            Self::write_world_time(&tx, datetime)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A completed player trade: both sides' state, both inventories and the
    /// ledger row in one commit. Separate from `save_batch` because the trade
    /// must be durable before either client is told it succeeded, and because
    /// the ledger row has to share the transaction to stay truthful.
    pub fn commit_trade(
        &self,
        characters: &[CharacterSaveData],
        inventories: &[(i64, Vec<ItemRow>)],
        ledger: &TradeLedgerEntry,
    ) -> Result<(), AuthError> {
        let conn = self.open_connection()?;
        let tx = conn.unchecked_transaction()?;
        Self::write_character_states(&tx, characters)?;
        Self::replace_inventories(
            &tx,
            inventories
                .iter()
                .map(|(id, items)| (*id, items.as_slice())),
        )?;
        tx.execute(
            "INSERT INTO player_trades (
                a_character_id, b_character_id,
                a_gold_before, a_gold_after, b_gold_before, b_gold_after,
                a_items, b_items
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ledger.a_character_id,
                ledger.b_character_id,
                ledger.a_gold_before,
                ledger.a_gold_after,
                ledger.b_gold_before,
                ledger.b_gold_after,
                ledger.a_items,
                ledger.b_items,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_world_time(&self) -> Result<Option<GameDateTime>, AuthError> {
        let conn = self.open_connection()?;
        Ok(conn
            .query_row(
                "SELECT year, month, day, hour, minute FROM world_time WHERE id = 1",
                [],
                |row| {
                    Ok(GameDateTime {
                        year: row.get(0)?,
                        month: row.get(1)?,
                        day: row.get(2)?,
                        hour: row.get(3)?,
                        minute: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn save_world_time(&self, datetime: &GameDateTime) -> Result<(), AuthError> {
        let conn = self.open_connection()?;
        Self::write_world_time(&conn, datetime)?;
        Ok(())
    }

    /// Load all items for a character.
    pub fn load_inventory(&self, character_id: i64) -> Result<Vec<ItemRow>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT item_def_id, quantity, equip_slot, enchant FROM character_items WHERE character_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![character_id], |row| {
                Ok(ItemRow {
                    item_def_id: row.get(0)?,
                    quantity: row.get(1)?,
                    equip_slot: row.get(2)?,
                    enchant: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Load all trained skills for a character. Missing rows mean level 0.
    pub fn load_skills(&self, character_id: i64) -> Result<Vec<SkillRow>, AuthError> {
        let conn = self.open_connection()?;
        let mut stmt = conn
            .prepare("SELECT skill_id, level, xp FROM character_skills WHERE character_id = ?1")?;
        let rows = stmt
            .query_map(params![character_id], |row| {
                Ok(SkillRow {
                    skill_id: row.get(0)?,
                    level: row.get(1)?,
                    xp: row.get::<_, i64>(2)? as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bard_starts_with_a_worn_mandolin_on_top_of_the_common_kit() {
        let db_path =
            std::env::temp_dir().join(format!("onlinerpg_auth_bard_{}.db", uuid::Uuid::new_v4()));
        let auth = AuthService::new(db_path).unwrap();
        let account = auth.login_google("sub-bard").unwrap();
        let attributes = CharacterAttributes {
            r#str: 10,
            dex: 12,
            con: 10,
            int: 10,
            wis: 10,
            cha: 14,
            guard: 0,
        };

        let bard = auth
            .create_character(
                &account,
                "Lark",
                &attributes,
                12,
                CharacterClass::Bard,
                Gender::Female,
            )
            .unwrap();
        let items = auth.load_inventory(bard.id).unwrap();
        assert!(items.iter().any(|r| r.item_def_id == "worn_mandolin"));
        assert!(items.iter().any(|r| r.item_def_id == "worn_iron_sword"));

        let knight = auth
            .create_character(
                &account,
                "Tass",
                &attributes,
                16,
                CharacterClass::Knight,
                Gender::Male,
            )
            .unwrap();
        let items = auth.load_inventory(knight.id).unwrap();
        assert!(items.iter().all(|r| r.item_def_id != "worn_mandolin"));
    }

    #[test]
    fn a_registry_npc_with_a_loadout_skips_the_starter_kit() {
        let db_path = std::env::temp_dir().join(format!(
            "onlinerpg_auth_loadout_{}.db",
            uuid::Uuid::new_v4()
        ));
        let auth = AuthService::new(db_path).unwrap();
        let account = auth.login_npc("npc_loadout_test").unwrap();
        let def = crate::npc_defs::npc_defs().get_by_npc_name("Karl").unwrap();
        assert!(
            !def.loadout.is_empty(),
            "Karl's registry row carries a loadout"
        );
        let attributes = CharacterAttributes {
            r#str: 15,
            dex: 13,
            con: 14,
            int: 9,
            wis: 11,
            cha: 10,
            guard: 11,
        };

        let karl = auth
            .create_character(
                &account,
                "Karl",
                &attributes,
                20,
                CharacterClass::Guard,
                Gender::Male,
            )
            .unwrap();
        let items = auth.load_inventory(karl.id).unwrap();
        assert!(
            items.is_empty(),
            "issued gear comes from join-time seeding, not creation: {items:?}"
        );
    }

    #[test]
    fn npc_login_enforces_prefix_and_google_separation() {
        let db_path =
            std::env::temp_dir().join(format!("onlinerpg_auth_npc_{}.db", uuid::Uuid::new_v4()));
        let auth = AuthService::new(db_path.clone()).unwrap();

        assert!(auth.login_npc("merchant_bob").is_err());
        assert!(auth.login_npc("").is_err());
        assert_eq!(auth.login_npc("npc_bob").unwrap(), "npc_bob");

        let player = auth.login_google("sub-123").unwrap();
        assert!(player.starts_with("player_"));
        assert!(auth.login_npc(&player).is_err());

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE accounts SET google_sub = 'sub-999' WHERE player_name = 'npc_bob'",
            [],
        )
        .unwrap();
        assert!(auth.login_npc("npc_bob").is_err());
    }

    #[test]
    fn a_ban_survives_recreating_the_character_and_expires_on_its_own() {
        let db_path =
            std::env::temp_dir().join(format!("onlinerpg_auth_ban_{}.db", uuid::Uuid::new_v4()));
        let auth = AuthService::new(db_path).unwrap();
        let account = auth.login_google("sub-ban").unwrap();
        let attributes = CharacterAttributes {
            r#str: 12,
            dex: 12,
            con: 12,
            int: 12,
            wis: 12,
            cha: 12,
            guard: 10,
        };
        let record = auth
            .create_character(
                &account,
                "Ruffian",
                &attributes,
                16,
                CharacterClass::Knight,
                Gender::Male,
            )
            .unwrap();

        // An operator types a character name; the ban lands on the account.
        assert_eq!(
            auth.account_of_character("ruffian").unwrap(),
            Some(("Ruffian".to_string(), account.clone())),
            "resolved ignoring case, like every other name lookup"
        );
        assert!(auth.active_ban(&account).unwrap().is_none());

        auth.ban_account(&account, Some("griefing"), None).unwrap();
        let ban = auth.active_ban(&account).unwrap().expect("ban in force");
        assert_eq!(ban.reason.as_deref(), Some("griefing"));
        assert_eq!(ban.until_unix, None, "no minutes means permanent");

        // Deleting the character does not shed the ban — the account carries it.
        auth.delete_character(&account, record.id).unwrap();
        assert!(auth.active_ban(&account).unwrap().is_some());
        assert_eq!(
            auth.login_google("sub-ban").unwrap(),
            account,
            "the same Google subject still resolves to the banned account"
        );

        // Re-banning replaces the row, so a permanent ban can be shortened.
        let until = unix_now() + 600;
        auth.ban_account(&account, Some("cooling off"), Some(until))
            .unwrap();
        let ban = auth.active_ban(&account).unwrap().expect("timed ban");
        assert_eq!(ban.until_unix, Some(until));
        assert!(ban.message().contains("10 minute"), "{}", ban.message());

        // A ban whose deadline has passed reads as absent and is swept.
        auth.ban_account(&account, None, Some(unix_now() - 1))
            .unwrap();
        assert!(auth.active_ban(&account).unwrap().is_none());
        assert!(
            !auth.unban_account(&account).unwrap(),
            "the expired row was cleared on read, so there is nothing left to lift"
        );

        // Lifting a live ban reports that it did something, once.
        auth.ban_account(&account, None, None).unwrap();
        assert!(auth.unban_account(&account).unwrap());
        assert!(!auth.unban_account(&account).unwrap());
        assert!(auth.active_ban(&account).unwrap().is_none());
    }

    /// The expiry path cleans up after itself, and a fresh ban placed after an
    /// expiry is honoured rather than swallowed by the cleanup.
    #[test]
    fn an_expired_ban_is_swept_and_a_later_ban_still_applies() {
        let db_path = std::env::temp_dir().join(format!(
            "onlinerpg_auth_ban_sweep_{}.db",
            uuid::Uuid::new_v4()
        ));
        let auth = AuthService::new(db_path).unwrap();
        let account = auth.login_npc("npc_ban_sweep").unwrap();

        auth.ban_account(&account, None, Some(unix_now() - 1))
            .unwrap();
        assert!(
            auth.active_ban(&account).unwrap().is_none(),
            "an expired ban stops applying"
        );
        assert!(
            !auth.unban_account(&account).unwrap(),
            "and the row is gone, so there is nothing left to lift"
        );

        auth.ban_account(&account, Some("re-banned"), None).unwrap();
        let ban = auth
            .active_ban(&account)
            .unwrap()
            .expect("the new ban applies");
        assert_eq!(ban.reason.as_deref(), Some("re-banned"));
    }

    /// A ban outlives its characters, so `/unban` has to be able to name the
    /// account directly — otherwise deleting the last character strands it.
    #[test]
    fn an_account_can_be_unbanned_after_its_last_character_is_gone() {
        let db_path = std::env::temp_dir().join(format!(
            "onlinerpg_auth_ban_orphan_{}.db",
            uuid::Uuid::new_v4()
        ));
        let auth = AuthService::new(db_path).unwrap();
        let account = auth.login_npc("npc_ban_orphan").unwrap();
        let record = auth
            .create_character(
                &account,
                "Lonely",
                &CharacterAttributes {
                    r#str: 12,
                    dex: 12,
                    con: 12,
                    int: 12,
                    wis: 12,
                    cha: 12,
                    guard: 10,
                },
                16,
                CharacterClass::Knight,
                Gender::Male,
            )
            .unwrap();
        auth.ban_account(&account, None, None).unwrap();
        auth.delete_character(&account, record.id).unwrap();

        // The character route is gone...
        assert!(auth.account_of_character("Lonely").unwrap().is_none());
        // ...but the ban is still there, and the account name still lifts it.
        assert!(auth.active_ban(&account).unwrap().is_some());
        assert!(auth.unban_account(&account).unwrap());
        assert!(auth.active_ban(&account).unwrap().is_none());
    }

    #[test]
    fn banning_an_unknown_character_finds_no_account() {
        let db_path = std::env::temp_dir().join(format!(
            "onlinerpg_auth_ban_miss_{}.db",
            uuid::Uuid::new_v4()
        ));
        let auth = AuthService::new(db_path).unwrap();
        assert!(auth.account_of_character("nobody").unwrap().is_none());
    }

    #[test]
    fn startup_migrates_legacy_item_definition_ids() {
        for (old, new, slot, enchant) in [
            ("leather_cap", "leather_helmet", "head", 2),
            ("iron_chestplate", "breastplate", "chest", 3),
        ] {
            let db_path = std::env::temp_dir().join(format!(
                "onlinerpg_auth_item_ids_{}.db",
                uuid::Uuid::new_v4()
            ));
            drop(AuthService::new(db_path.clone()).unwrap());

            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "INSERT INTO accounts (player_name) VALUES ('legacy');
                 INSERT INTO characters (id, account_name, character_name)
                 VALUES (1, 'legacy', 'LegacyWearer');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO character_items
                 (character_id, item_def_id, quantity, equip_slot, enchant)
                 VALUES (1, ?1, 1, ?2, ?3)",
                params![old, slot, enchant],
            )
            .unwrap();
            drop(conn);

            let auth = AuthService::new(db_path).unwrap();
            let rows = auth.load_inventory(1).unwrap();
            assert_eq!(rows.len(), 1, "migrating {old}");
            assert_eq!(rows[0].item_def_id, new);
            assert_eq!(rows[0].equip_slot.as_deref(), Some(slot));
            assert_eq!(rows[0].enchant, enchant);
        }
    }

    /// A typo'd rename target would silently hand players a ghost item:
    /// unknown ids survive inventory load and fall back to the raw id string.
    #[test]
    fn renamed_item_ids_resolve_to_real_defs() {
        let defs = crate::item_defs::ItemDefs::load();
        for &(old, new) in RENAMED_ITEM_IDS {
            assert!(defs.get(new).is_some(), "{old} renamed to unknown id {new}");
        }
    }

    #[test]
    fn skills_round_trip_and_upsert_preserves_unknown_rows() {
        let db_path =
            std::env::temp_dir().join(format!("onlinerpg_auth_skills_{}.db", uuid::Uuid::new_v4()));
        let auth = AuthService::new(db_path).unwrap();
        let account = auth.login_npc("npc_skills_test").unwrap();
        let attributes = CharacterAttributes {
            r#str: 12,
            dex: 12,
            con: 12,
            int: 12,
            wis: 12,
            cha: 12,
            guard: 10,
        };
        let record = auth
            .create_character(
                &account,
                "Fisherman",
                &attributes,
                16,
                CharacterClass::Ranger,
                Gender::Female,
            )
            .unwrap();

        // Fresh character: no rows.
        assert!(auth.load_skills(record.id).unwrap().is_empty());

        // A row a "newer server" wrote must survive our saves (upsert, no delete).
        auth.save_batch(
            &[],
            &[],
            &[(
                record.id,
                vec![SkillRow {
                    skill_id: "underwater_basketweaving".to_string(),
                    level: 7,
                    xp: 999,
                }],
            )],
            &[],
            None,
        )
        .unwrap();

        auth.save_batch(
            &[],
            &[],
            &[(
                record.id,
                vec![SkillRow {
                    skill_id: "fishing".to_string(),
                    level: 2,
                    xp: 500,
                }],
            )],
            &[],
            None,
        )
        .unwrap();

        let mut rows = auth.load_skills(record.id).unwrap();
        rows.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].skill_id, "fishing");
        assert_eq!(rows[0].level, 2);
        assert_eq!(rows[0].xp, 500);
        assert_eq!(rows[1].skill_id, "underwater_basketweaving");
        assert_eq!(rows[1].xp, 999);

        // Advancing a skill updates in place rather than duplicating the row.
        auth.save_batch(
            &[],
            &[],
            &[(
                record.id,
                vec![SkillRow {
                    skill_id: "fishing".to_string(),
                    level: 3,
                    xp: 1400,
                }],
            )],
            &[],
            None,
        )
        .unwrap();
        let rows = auth.load_skills(record.id).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().find(|r| r.skill_id == "fishing").unwrap().xp,
            1400
        );
    }

    /// EnterGame refuses the session when this load errs, so a missing table
    /// must surface as an error — never as a valid empty history.
    #[test]
    fn dungeon_history_load_fails_when_storage_is_unavailable() {
        let db_path = std::env::temp_dir().join(format!(
            "onlinerpg_auth_dungeon_history_{}.db",
            uuid::Uuid::new_v4()
        ));
        let auth = AuthService::new(db_path).unwrap();
        let (opens, discoveries) = auth.load_dungeon_history(1).unwrap();
        assert!(opens.is_empty());
        assert!(discoveries.is_empty());

        for table in ["character_dungeon_chests", "character_dungeon_discoveries"] {
            let db_path = std::env::temp_dir().join(format!(
                "onlinerpg_auth_dungeon_history_{}.db",
                uuid::Uuid::new_v4()
            ));
            let auth = AuthService::new(db_path.clone()).unwrap();
            let conn = Connection::open(db_path).unwrap();
            conn.execute(&format!("DROP TABLE {table}"), []).unwrap();

            assert!(auth.load_dungeon_history(1).is_err());
        }
    }

    #[test]
    fn name_validation_rejects_control_chars_and_long_names() {
        let db_path =
            std::env::temp_dir().join(format!("onlinerpg_auth_names_{}.db", uuid::Uuid::new_v4()));
        let auth = AuthService::new(db_path).unwrap();

        assert!(auth.login_npc("npc_bad\nname").is_err());

        let account = auth.login_npc("npc_name_test").unwrap();
        let attributes = CharacterAttributes {
            r#str: 12,
            dex: 12,
            con: 12,
            int: 12,
            wis: 12,
            cha: 12,
            guard: 10,
        };
        let create = |name: &str| {
            auth.create_character(
                &account,
                name,
                &attributes,
                16,
                CharacterClass::Knight,
                Gender::Male,
            )
        };
        assert!(create("Bad\nName").is_err());
        assert!(create("30000").is_err());
        assert!(create("_Player").is_err());
        assert!(create("김철수").is_ok());
        assert!(create("ㅇㅇ_Player1").is_ok());
        assert!(create("Player1").is_ok());

        assert!(!valid_character_name("30000"));
        assert!(!valid_character_name("9lives"));
        assert!(!valid_character_name("_x"));
        assert!(valid_character_name("x_9"));
        assert!(valid_character_name("가9"));

        assert!(!valid_name(""));
        assert!(!valid_name("Bad\u{1b}[31mName"));
        assert!(!valid_name("Zero\u{200b}Width"));
        assert!(!valid_name("Emo😀ji"));
        assert!(!valid_name("Rtl\u{202e}Name"));
        assert!(!valid_name("With Space"));
        assert!(!valid_name(&"x".repeat(33)));
        assert!(!valid_name("Ｆｕｌｌ"));
        assert!(!valid_name("ﾊﾝｶｸ"));
        assert!(!valid_name("・"));
        assert!(valid_name("さくら"));
        assert!(valid_name("ローラ"));
        assert!(valid_name("田中太郎"));
        assert!(valid_name("劍聖"));
    }

    #[test]
    fn character_names_unique_ignoring_ascii_case() {
        let db_path =
            std::env::temp_dir().join(format!("onlinerpg_auth_case_{}.db", uuid::Uuid::new_v4()));
        let auth = AuthService::new(db_path).unwrap();
        let account = auth.login_npc("npc_case_test").unwrap();

        let attributes = CharacterAttributes {
            r#str: 12,
            dex: 12,
            con: 12,
            int: 12,
            wis: 12,
            cha: 12,
            guard: 10,
        };
        let create = |name: &str| {
            auth.create_character(
                &account,
                name,
                &attributes,
                16,
                CharacterClass::Knight,
                Gender::Male,
            )
        };
        assert!(create("Valkyrie").is_ok());
        assert!(matches!(
            create("valkyrie"),
            Err(AuthError::CharacterNameAlreadyExists)
        ));
        assert!(matches!(
            create("VALKYRIE"),
            Err(AuthError::CharacterNameAlreadyExists)
        ));
    }

    #[test]
    fn startup_rejects_legacy_case_colliding_names() {
        let db_path =
            std::env::temp_dir().join(format!("onlinerpg_auth_legacy_{}.db", uuid::Uuid::new_v4()));
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE characters (
                    id INTEGER PRIMARY KEY,
                    account_name TEXT NOT NULL,
                    character_name TEXT NOT NULL UNIQUE,
                    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                    level INTEGER NOT NULL DEFAULT 1,
                    max_hp INTEGER NOT NULL DEFAULT 16,
                    attr_str INTEGER NOT NULL DEFAULT 12,
                    attr_dex INTEGER NOT NULL DEFAULT 12,
                    attr_con INTEGER NOT NULL DEFAULT 12,
                    attr_int INTEGER NOT NULL DEFAULT 12,
                    attr_wis INTEGER NOT NULL DEFAULT 12,
                    attr_cha INTEGER NOT NULL DEFAULT 12,
                    attr_guard INTEGER NOT NULL DEFAULT 10
                );
                INSERT INTO characters (account_name, character_name)
                VALUES ('legacy', 'Bob'), ('legacy', 'bob');",
            )
            .unwrap();
        }

        // The unique NOCASE index cannot be built over colliding rows;
        // startup fails until the offending characters are renamed.
        assert!(AuthService::new(db_path).is_err());
    }

    #[test]
    fn admin_role_defaults_to_zero_and_loads_after_update() {
        let db_path =
            std::env::temp_dir().join(format!("onlinerpg_auth_admin_{}.db", uuid::Uuid::new_v4()));
        let auth = AuthService::new(db_path.clone()).unwrap();
        let account = auth.login_npc("npc_admin_role_test").unwrap();

        let attributes = CharacterAttributes {
            r#str: 12,
            dex: 12,
            con: 12,
            int: 12,
            wis: 12,
            cha: 12,
            guard: 10,
        };
        let record = auth
            .create_character(
                &account,
                "AdminRoleTest",
                &attributes,
                16,
                CharacterClass::Knight,
                Gender::Male,
            )
            .unwrap();
        assert_eq!(record.admin_role, 0);

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE characters SET admin_role = 2 WHERE id = ?1",
            params![record.id],
        )
        .unwrap();

        let loaded = auth.get_character_for_account(&account, record.id).unwrap();
        assert_eq!(loaded.admin_role, 2);
    }
}

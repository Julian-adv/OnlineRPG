use super::GameState;
use crate::types::{Player, PlayerId};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const MAX_PENDING: usize = 16_384;
const MAX_MONSTERS: usize = 4_096;
const CONFIG_RELOAD_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Config {
    character_ids: HashSet<i64>,
    retention_days: u16,
}

#[derive(Default, Serialize)]
struct MonsterTotals {
    server_attempts: u64,
    client_requests: u64,
    rejected: BTreeMap<String, u64>,
    hits: u64,
    misses: u64,
    damage: u64,
    kills: u64,
    kills_without_observed_attempt: u64,
}

#[derive(Serialize)]
struct Window {
    schema: u8,
    character_id: i64,
    player_id: PlayerId,
    name: String,
    start_ms: u64,
    end_ms: u64,
    reason: &'static str,
    start_hp: u32,
    end_hp: u32,
    max_hp: u32,
    level: u32,
    health_gained: BTreeMap<String, u64>,
    health_lost: BTreeMap<String, u64>,
    deaths: u64,
    level_ups: u64,
    history_overflow: bool,
    monsters: BTreeMap<String, MonsterTotals>,
}

impl Window {
    fn new(character_id: i64, p: &Player, now: u64) -> Self {
        Self {
            schema: 1,
            character_id,
            player_id: p.id,
            name: p.name.clone(),
            start_ms: now,
            end_ms: now,
            reason: "interval",
            start_hp: p.health,
            end_hp: p.health,
            max_hp: p.max_health,
            level: p.level,
            health_gained: BTreeMap::new(),
            health_lost: BTreeMap::new(),
            deaths: 0,
            level_ups: 0,
            history_overflow: false,
            monsters: BTreeMap::new(),
        }
    }

    fn health(&mut self, old: u32, p: &Player, source: &str) {
        if p.health > old {
            *self.health_gained.entry(source.into()).or_default() += u64::from(p.health - old);
        } else if p.health < old {
            *self.health_lost.entry(source.into()).or_default() += u64::from(old - p.health);
        }
        self.end_hp = p.health;
        self.max_hp = p.max_health;
        self.level = p.level;
    }

    fn record_attempt(&mut self, kind: Option<&str>, client: bool) -> &mut MonsterTotals {
        let counts = self
            .monsters
            .entry(kind.unwrap_or("unknown").into())
            .or_default();
        if client {
            counts.client_requests += 1;
        } else {
            counts.server_attempts += 1;
        }
        counts
    }
}

struct Session {
    window: Window,
    attempted: HashSet<String>,
}

#[derive(Default)]
struct State {
    config: Option<Config>,
    config_error: Option<String>,
    next_config_read: Option<tokio::time::Instant>,
    characters: HashMap<PlayerId, i64>,
    sessions: HashMap<PlayerId, Session>,
    pending: VecDeque<Window>,
    last_pruned: Option<(String, u16)>,
}

impl State {
    fn prune_due(&self, now: u64) -> Option<(String, u16)> {
        let key = (date(now), self.config.as_ref()?.retention_days);
        (self.last_pruned.as_ref() != Some(&key)).then_some(key)
    }

    fn queue(&mut self, mut row: Window, now: u64, reason: &'static str) {
        row.end_ms = now;
        row.reason = reason;
        if self.pending.len() == MAX_PENDING {
            self.pending.pop_front();
            tracing::error!("Combat audit queue full; oldest interval lost");
        }
        self.pending.push_back(row);
    }

    fn observe(&mut self, p: &Player, now: u64) {
        let Some(&character_id) = self.characters.get(&p.id) else {
            return;
        };
        if self
            .config
            .as_ref()
            .is_some_and(|c| c.character_ids.contains(&character_id))
        {
            self.sessions.entry(p.id).or_insert_with(|| Session {
                window: Window::new(character_id, p, now),
                attempted: HashSet::new(),
            });
        }
    }
}

#[derive(Default)]
pub(super) struct CombatAudit {
    active: AtomicBool,
    state: Mutex<State>,
    io_lock: Mutex<()>,
}

impl CombatAudit {
    pub(super) fn register(&self, id: PlayerId, character_id: i64) {
        self.state
            .lock()
            .expect("audit state")
            .characters
            .insert(id, character_id);
    }

    pub(super) fn observe(&self, p: &Player) {
        if self.active.load(Ordering::Relaxed) {
            self.state
                .lock()
                .expect("audit state")
                .observe(p, GameState::now_ms());
        }
    }

    pub(super) fn logout(&self, id: &PlayerId) {
        let mut state = self.state.lock().expect("audit state");
        state.characters.remove(id);
        if let Some(session) = state.sessions.remove(id) {
            state.queue(session.window, GameState::now_ms(), "logout");
        }
    }

    pub(super) fn health(&self, old: u32, p: &Player, source: &str) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        if let Some(s) = self
            .state
            .lock()
            .expect("audit state")
            .sessions
            .get_mut(&p.id)
        {
            s.window.health(old, p, source);
            if source == "level_up" {
                s.window.level_ups += 1;
            }
        }
    }

    pub(super) fn death(&self, id: &PlayerId) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        if let Some(s) = self.state.lock().expect("audit state").sessions.get_mut(id) {
            s.window.deaths += 1;
        }
    }

    pub(super) fn kill(&self, id: &PlayerId, monster_id: &str, kind: &str) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        let mut state = self.state.lock().expect("audit state");
        for (pid, s) in &mut state.sessions {
            let attempted = s.attempted.remove(monster_id);
            if pid == id {
                let counts = s.window.monsters.entry(kind.into()).or_default();
                counts.kills += 1;
                if !attempted && !s.window.history_overflow {
                    counts.kills_without_observed_attempt += 1;
                }
            }
        }
    }

    pub(super) fn attack<'a>(&'a self, id: PlayerId, client: bool) -> Attack<'a> {
        Attack {
            audit: self,
            id,
            client,
            kind: None,
            reason: "missing_monster",
            finished: false,
        }
    }

    fn refresh(
        &self,
        config: Option<Config>,
        players: &HashMap<PlayerId, Player>,
        now: u64,
        finish: bool,
    ) {
        let mut state = self.state.lock().expect("audit state");
        let mut config_changed = false;
        if let Some(config) = config {
            if state.config.as_ref() != Some(&config) {
                tracing::info!(character_ids = ?config.character_ids, retention_days = config.retention_days, "Combat audit targets updated (manual removal only)");
                state.config = Some(config);
                config_changed = true;
            }
        }
        let targets = state
            .config
            .as_ref()
            .map(|c| c.character_ids.clone())
            .unwrap_or_default();
        self.active.store(!targets.is_empty(), Ordering::Relaxed);
        let ids: Vec<_> = state.sessions.keys().copied().collect();
        for id in ids {
            let session = &state.sessions[&id];
            let enabled = targets.contains(&session.window.character_id);
            if finish || !enabled || now.saturating_sub(session.window.start_ms) >= 60_000 {
                let mut session = state.sessions.remove(&id).expect("audit session");
                if let Some(p) = players.get(&id) {
                    session.window.end_hp = p.health;
                    session.window.max_hp = p.max_health;
                    session.window.level = p.level;
                }
                let reason = if finish {
                    "shutdown"
                } else if !enabled {
                    "disabled"
                } else {
                    "interval"
                };
                if enabled && !finish {
                    if let Some(p) = players.get(&id) {
                        let mut window = Window::new(session.window.character_id, p, now);
                        window.history_overflow = session.window.history_overflow;
                        state.sessions.insert(
                            id,
                            Session {
                                window,
                                attempted: session.attempted,
                            },
                        );
                    }
                }
                state.queue(session.window, now, reason);
            }
        }
        if !finish && config_changed {
            for p in players.values() {
                state.observe(p, now);
            }
        }
    }

    fn flush(&self, dir: &Path) -> io::Result<()> {
        let _writer = self.io_lock.lock().expect("audit writer");
        {
            let state = self.state.lock().expect("audit state");
            if state.config.is_none() && state.pending.is_empty() {
                return Ok(());
            }
        }
        fs::create_dir_all(dir)?;
        loop {
            let row = self.state.lock().expect("audit state").pending.pop_front();
            let Some(row) = row else { break };
            let result = write_row(dir, &row);
            if let Err(err) = result {
                let mut state = self.state.lock().expect("audit state");
                if state.pending.len() == MAX_PENDING {
                    state.pending.pop_back();
                    tracing::error!("Combat audit queue full during retry; newest interval lost");
                }
                state.pending.push_front(row);
                return Err(err);
            }
        }
        let now = GameState::now_ms();
        let prune_due = self.state.lock().expect("audit state").prune_due(now);
        if let Some(key) = prune_due {
            prune(dir, now, key.1)?;
            self.state.lock().expect("audit state").last_pruned = Some(key);
        }
        Ok(())
    }
}

pub(super) struct Attack<'a> {
    audit: &'a CombatAudit,
    id: PlayerId,
    client: bool,
    kind: Option<String>,
    pub(super) reason: &'static str,
    finished: bool,
}

impl Attack<'_> {
    pub(super) fn monster(&mut self, id: &str, kind: &str) {
        if !self.audit.active.load(Ordering::Relaxed) {
            return;
        }
        if let Some(s) = self
            .audit
            .state
            .lock()
            .expect("audit state")
            .sessions
            .get_mut(&self.id)
        {
            self.kind = Some(kind.into());
            if s.attempted.len() < MAX_MONSTERS {
                s.attempted.insert(id.into());
            } else if !s.attempted.contains(id) {
                s.window.history_overflow = true;
            }
        }
    }

    pub(super) fn resolved(&mut self, old: u32, p: &Player, hit: bool) {
        self.finished = true;
        if !self.audit.active.load(Ordering::Relaxed) {
            return;
        }
        if let Some(s) = self
            .audit
            .state
            .lock()
            .expect("audit state")
            .sessions
            .get_mut(&self.id)
        {
            s.window.health(old, p, "monster");
            let counts = s.window.record_attempt(self.kind.as_deref(), self.client);
            if hit {
                counts.hits += 1;
            } else {
                counts.misses += 1;
            }
            counts.damage += u64::from(old.saturating_sub(p.health));
        }
    }
}

impl Drop for Attack<'_> {
    fn drop(&mut self) {
        if self.finished || !self.audit.active.load(Ordering::Relaxed) {
            return;
        }
        if let Some(s) = self
            .audit
            .state
            .lock()
            .expect("audit state")
            .sessions
            .get_mut(&self.id)
        {
            let counts = s.window.record_attempt(self.kind.as_deref(), self.client);
            *counts.rejected.entry(self.reason.into()).or_default() += 1;
        }
    }
}

fn read_config(path: &Path, retention_days: u16) -> io::Result<Config> {
    if fs::metadata(path)?.len() > 65_536 {
        return Err(io::Error::other("audit config too large"));
    }
    let text = fs::read_to_string(path)?;
    if retention_days == 0 {
        return Err(io::Error::other("audit retention_days must be positive"));
    }
    let mut character_ids = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let id = line
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(|| {
                io::Error::other(format!("invalid character ID on line {}", index + 1))
            })?;
        character_ids.insert(id);
        if character_ids.len() > 128 {
            return Err(io::Error::other("audit supports at most 128 characters"));
        }
    }
    Ok(Config {
        character_ids,
        retention_days,
    })
}

fn date(ms: u64) -> String {
    let dt =
        time::OffsetDateTime::from_unix_timestamp((ms / 1000) as i64).expect("audit timestamp");
    format!(
        "{:04}-{:02}-{:02}",
        dt.year(),
        u8::from(dt.month()),
        dt.day()
    )
}

fn write_row(dir: &Path, row: &Window) -> io::Result<()> {
    let path = dir.join(format!("combat-audit-{}.jsonl", date(row.start_ms)));
    let mut bytes = serde_json::to_vec(row)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let original_len = file.metadata()?.len();
    if let Err(err) = file.write_all(&bytes).and_then(|()| file.sync_data()) {
        file.set_len(original_len)?;
        return Err(err);
    }
    Ok(())
}

fn prune(dir: &Path, now: u64, days: u16) -> io::Result<()> {
    let first_kept = date(now.saturating_sub((u64::from(days) - 1) * 86_400_000));
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(day) = name
            .strip_prefix("combat-audit-")
            .and_then(|s| s.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if day.len() == 10
            && day.as_bytes()[4] == b'-'
            && day.as_bytes()[7] == b'-'
            && day
                .chars()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
            && day < first_kept.as_str()
            && entry.file_type()?.is_file()
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

impl GameState {
    pub async fn tick_combat_audit(&self, state_dir: PathBuf, retention_days: u16, finish: bool) {
        let reload = !finish && {
            let now = tokio::time::Instant::now();
            let mut state = self.combat_audit.state.lock().expect("audit state");
            if state.next_config_read.is_none_or(|next| now >= next) {
                state.next_config_read = Some(now + CONFIG_RELOAD_INTERVAL);
                true
            } else {
                false
            }
        };
        let config = if reload {
            let path = state_dir.join("combat-audit.txt");
            let result =
                tokio::task::spawn_blocking(move || read_config(&path, retention_days)).await;
            let result = result.unwrap_or_else(|err| Err(io::Error::other(err.to_string())));
            {
                let mut state = self.combat_audit.state.lock().expect("audit state");
                match result {
                    Ok(config) => {
                        state.config_error = None;
                        Some(config)
                    }
                    Err(err) => {
                        let message = err.to_string();
                        if (err.kind() != io::ErrorKind::NotFound || state.config.is_some())
                            && state.config_error.as_ref() != Some(&message)
                        {
                            tracing::warn!(
                                "Combat audit config unreadable; retaining current targets: {err}"
                            );
                        }
                        state.config_error = Some(message);
                        None
                    }
                }
            }
        } else {
            None
        };
        {
            let players = self.players.read().await;
            self.combat_audit
                .refresh(config, &players, Self::now_ms(), finish);
        }
        {
            let monsters = self.monsters.read().await;
            let mut state = self.combat_audit.state.lock().expect("audit state");
            for s in state.sessions.values_mut() {
                s.attempted.retain(|id| monsters.get(id).is_some());
            }
        }
        {
            let state = self.combat_audit.state.lock().expect("audit state");
            if state.pending.is_empty() && state.prune_due(Self::now_ms()).is_none() {
                return;
            }
        }
        let audit = self.combat_audit.clone();
        match tokio::task::spawn_blocking(move || audit.flush(&state_dir.join("combat-audit")))
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                tracing::error!("Combat audit write failed; queued intervals will retry: {err}")
            }
            Err(err) => tracing::error!("Combat audit writer task failed: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::tests::make_player;

    fn setup() -> (CombatAudit, HashMap<PlayerId, Player>) {
        let audit = CombatAudit::default();
        let player = make_player("watched", 0.0, 0.0);
        audit.register(player.id, 6229);
        let players = HashMap::from([(player.id, player)]);
        audit.refresh(
            Some(Config {
                character_ids: HashSet::from([6229]),
                retention_days: 30,
            }),
            &players,
            1000,
            false,
        );
        (audit, players)
    }

    #[test]
    fn targets_do_not_expire_and_attempt_history_survives_intervals() {
        let (audit, players) = setup();
        let p = players.values().next().unwrap();
        {
            let mut attack = audit.attack(p.id, false);
            attack.monster("m1", "troll");
            attack.resolved(p.health, p, false);
        }
        audit.refresh(None, &players, 61_000, false);
        audit.kill(&p.id, "m1", "troll");
        audit.kill(&p.id, "m2", "troll");
        audit.refresh(None, &players, 3 * 86_400_000, false);
        let state = audit.state.lock().unwrap();
        assert!(state.sessions.contains_key(&p.id));
        let first = &state.pending[0].monsters["troll"];
        assert_eq!((first.server_attempts, first.misses), (1, 1));
        let second = &state.pending[1].monsters["troll"];
        assert_eq!(
            (second.kills, second.kills_without_observed_attempt),
            (2, 1)
        );
        drop(state);
        audit.refresh(
            Some(Config {
                character_ids: HashSet::new(),
                retention_days: 30,
            }),
            &players,
            3 * 86_400_000 + 1,
            false,
        );
        let state = audit.state.lock().unwrap();
        assert!(state.sessions.is_empty());
        assert_eq!(state.pending.back().unwrap().reason, "disabled");
    }

    #[test]
    fn actual_health_balances_even_with_overkill_and_full_heal() {
        let (audit, mut players) = setup();
        let p = players.values_mut().next().unwrap();
        {
            let mut attack = audit.attack(p.id, false);
            attack.monster("m1", "troll");
            p.health = 0;
            attack.resolved(10, p, true);
        }
        audit.death(&p.id);
        p.health = 10;
        audit.health(0, p, "revive");
        p.max_health = 15;
        p.health = 15;
        audit.health(10, p, "level_up");
        audit.refresh(None, &players, 61_000, true);
        let state = audit.state.lock().unwrap();
        let row = &state.pending[0];
        assert_eq!(row.monsters["troll"].damage, 10);
        assert_eq!(row.health_lost["monster"], 10);
        assert_eq!(row.health_gained["revive"], 10);
        assert_eq!(row.health_gained["level_up"], 5);
        assert_eq!((row.deaths, row.level_ups), (1, 1));
        assert_eq!(
            i64::from(row.end_hp) - i64::from(row.start_hp),
            row.health_gained.values().sum::<u64>() as i64
                - row.health_lost.values().sum::<u64>() as i64
        );
    }

    #[test]
    fn history_overflow_is_explicit_and_never_claims_zero_attempts() {
        let (audit, players) = setup();
        let p = players.values().next().unwrap();
        for i in 0..=MAX_MONSTERS {
            let mut attack = audit.attack(p.id, false);
            attack.monster(&format!("m{i}"), "troll");
        }
        audit.kill(&p.id, "unknown", "troll");
        audit.refresh(None, &players, 61_000, true);
        let state = audit.state.lock().unwrap();
        let row = &state.pending[0];
        assert!(row.history_overflow);
        assert_eq!(row.monsters["troll"].kills_without_observed_attempt, 0);
    }

    #[test]
    fn invalid_config_is_rejected_and_file_survives_a_new_instance() {
        let dir = crate::test_util::unique_temp_dir("audit_config");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("combat-audit.txt");
        fs::write(&path, "# Watched characters\r\n 6229 \r\n\r\n6229\n").unwrap();
        for _ in 0..2 {
            let (audit, players) = setup();
            let config = read_config(&path, 30).unwrap();
            assert_eq!(config.character_ids, HashSet::from([6229]));
            audit.refresh(Some(config), &players, 1000, false);
            assert_eq!(audit.state.lock().unwrap().sessions.len(), 1);
        }
        assert!(read_config(&path, 0).is_err());
        for invalid in [
            "-1",
            "0",
            "6229\nExamplePlayer",
            "6229,6230",
            "9223372036854775808",
        ] {
            fs::write(&path, invalid).unwrap();
            assert!(read_config(&path, 30).is_err());
        }
        fs::write(&path, "6229\n6230\n").unwrap();
        assert_eq!(
            read_config(&path, 7).unwrap().character_ids,
            HashSet::from([6229, 6230])
        );
        fs::write(&path, "\n# Nobody\n").unwrap();
        assert!(read_config(&path, 30).unwrap().character_ids.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_write_retries_without_losing_or_duplicating_rows() {
        let (audit, players) = setup();
        audit.refresh(None, &players, 61_000, true);
        let dir = crate::test_util::unique_temp_dir("audit_retry");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("combat-audit-1970-01-01.jsonl");
        fs::create_dir(&path).unwrap();
        assert!(audit.flush(&dir).is_err());
        assert_eq!(audit.state.lock().unwrap().pending.len(), 1);
        fs::remove_dir(&path).unwrap();
        audit.state.lock().unwrap().config = None;
        audit.flush(&dir).unwrap();
        audit.flush(&dir).unwrap();
        let output = fs::read_to_string(path).unwrap();
        assert_eq!(output.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output).unwrap()["reason"],
            "shutdown"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retention_only_removes_expired_audit_dates() {
        let dir = crate::test_util::unique_temp_dir("audit_retention");
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "combat-audit-1970-01-01.jsonl",
            "combat-audit-1970-01-02.jsonl",
            "combat-audit-1970-01-03.jsonl",
            "other.jsonl",
        ] {
            fs::write(dir.join(name), "{}").unwrap();
        }
        prune(&dir, 2 * 86_400_000, 2).unwrap();
        assert!(!dir.join("combat-audit-1970-01-01.jsonl").exists());
        assert!(dir.join("combat-audit-1970-01-02.jsonl").exists());
        assert!(dir.join("combat-audit-1970-01-03.jsonl").exists());
        assert!(dir.join("other.jsonl").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}

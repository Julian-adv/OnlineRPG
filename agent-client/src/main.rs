mod bgm_defs;
mod claude;
mod codex;
mod driver;
mod dungeon;
mod geom;
mod google_auth;
mod item_defs;
mod llm_scheduler;
mod monster_ai;
mod openai;
mod openrouter;
mod orchestrator;
mod shop_info;
mod state;
mod terrain_http;
mod watch;
mod ws;

use std::sync::Arc;
use std::time::Duration;

use onlinerpg_terrain::height::HeightSampler;
use onlinerpg_terrain::io::TerrainIO;
use orchestrator::{AuthSource, NpcConfig, SharedResources};
use serde::Deserialize;
use tracing::{info, warn};

/// Which LLM backend to use for the agent driver.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LlmType {
    /// No LLM driver: schedule and monster AI drive the NPC on their own.
    #[default]
    None,
    /// Claude CLI (stdio subprocess)
    Claude,
    /// OpenRouter API (HTTP)
    Openrouter,
    /// Codex CLI (stdio subprocess)
    Codex,
    /// Any OpenAI-compatible chat completions endpoint (HTTP)
    Openai,
}

/// Config parsed from TOML. Uses `[[npcs]]` array for multi-NPC orchestrator.
#[derive(Deserialize)]
struct Config {
    /// Server WebSocket URL
    server: String,
    /// NPC auth token; defaults to reading the server-generated
    /// ../data/npc_token file (same machine).
    npc_token: Option<String>,
    /// Where heightmap tiles come from: a local terrain directory, or an
    /// `http(s)://` server origin for clients running on another machine.
    #[serde(default = "default_terrain", alias = "terrain_dir")]
    terrain: String,
    /// Disk cache for tiles fetched over HTTP (ignored for a local source).
    #[serde(default = "default_terrain_cache")]
    terrain_cache: String,

    /// How this client authenticates to the game server.
    #[serde(default)]
    auth: AuthConfig,

    /// Array of NPC configurations.
    #[serde(default)]
    npcs: Vec<NpcConfig>,

    /// Maximum number of concurrent LLM calls across all NPCs (default: 2)
    #[serde(default = "default_max_concurrent")]
    max_concurrent: usize,

    /// Give up on one LLM call after this many seconds (default: 120, 0 = off).
    /// Applies to every backend: a hung CLI subprocess holds a `max_concurrent`
    /// slot exactly as long as an unanswered HTTP request does.
    #[serde(default = "default_request_timeout_secs")]
    request_timeout_secs: u64,

    /// Spectator panel port on 127.0.0.1 (default: 0, off)
    #[serde(default)]
    watch_port: u16,

    /// Claude CLI integration config (shared across NPCs that don't override)
    #[serde(default)]
    claude: claude::ClaudeConfig,
    /// OpenRouter API integration config
    #[serde(default)]
    openrouter: openrouter::OpenRouterConfig,
    /// Codex CLI integration config
    #[serde(default)]
    codex: codex::CodexConfig,
    /// Generic OpenAI-compatible endpoint config
    #[serde(default)]
    openai: openai::OpenAiConfig,
}

/// How the client proves who it is to the game server.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// Shared secret from the server's `data/npc_token` — operator-run NPCs
    /// on the server machine.
    #[default]
    NpcToken,
    /// The runner's own Google account (see `doc/REMOTE_AGENT_CLIENT.md`).
    Google,
}

#[derive(Debug, Default, Deserialize)]
struct AuthConfig {
    #[serde(default)]
    mode: AuthMode,
    /// Google OAuth settings; used when `mode = "google"`.
    #[serde(default, flatten)]
    google: google_auth::GoogleAuthConfig,
}

fn default_terrain() -> String {
    "../data/terrain".to_string()
}

fn default_terrain_cache() -> String {
    "data/cache/height".to_string()
}

fn is_http_source(terrain: &str) -> bool {
    terrain.starts_with("http://") || terrain.starts_with("https://")
}

pub fn default_min_interval_secs() -> u64 {
    5
}

pub fn default_urgent_min_interval_secs() -> u64 {
    2
}

pub fn default_debounce_secs() -> u64 {
    2
}

pub fn default_idle_interval_secs() -> u64 {
    3600
}

pub fn default_activity_window_secs() -> u64 {
    30
}

fn default_max_concurrent() -> usize {
    2
}

fn check_max_concurrent(max_concurrent: usize) -> anyhow::Result<()> {
    if max_concurrent == 0 {
        anyhow::bail!("max_concurrent must be at least 1");
    }
    Ok(())
}

fn default_request_timeout_secs() -> u64 {
    120
}

const CONFIG_PATH: &str = "data/config.toml";

fn resolve_npc_token(config_value: Option<String>) -> anyhow::Result<String> {
    if let Some(token) = config_value {
        return Ok(token);
    }
    // Server writes the token under its default state dir at the repo root;
    // our cwd is one level down.
    let path = format!("../data/{}", onlinerpg_shared::NPC_TOKEN_FILENAME);
    let token = std::fs::read_to_string(&path)
        .map_err(|e| {
            anyhow::anyhow!(
                "no npc_token in {CONFIG_PATH} and failed to read {path} \
                 (start the game server once to generate it): {e}"
            )
        })?
        .trim()
        .to_string();
    if token.is_empty() {
        anyhow::bail!("{path} is empty");
    }
    Ok(token)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_text = std::fs::read_to_string(CONFIG_PATH)
        .map_err(|e| anyhow::anyhow!("Failed to read {CONFIG_PATH}: {e}"))?;
    let config: Config = toml::from_str(&config_text)
        .map_err(|e| anyhow::anyhow!("Failed to parse {CONFIG_PATH}: {e}"))?;

    check_max_concurrent(config.max_concurrent)?;
    if config.npcs.is_empty() {
        anyhow::bail!("No [[npcs]] configured in {CONFIG_PATH}");
    }

    if config.auth.mode == AuthMode::Google {
        check_google_mode_config(&config.npcs)?;
    }

    // NPCs inherit root-level backend configs when they don't override them
    let mut npcs = config.npcs;
    for npc in &mut npcs {
        resolve_from_registry(npc)?;
        if npc.claude == claude::ClaudeConfig::default() {
            npc.claude = config.claude.clone();
        }
        if npc.openrouter == openrouter::OpenRouterConfig::default() {
            npc.openrouter = config.openrouter.clone();
        }
        if npc.codex == codex::CodexConfig::default() {
            npc.codex = config.codex.clone();
        }
        if npc.openai == openai::OpenAiConfig::default() {
            npc.openai = config.openai.clone();
        }
    }

    let behavior_trees = monster_ai::MonsterAiManager::load_behavior_trees_from_json(include_str!(
        "../../data-src/behavior_trees.json"
    ));
    let (type_mapping, movement_speeds) =
        monster_ai::MonsterAiManager::load_monster_data(include_str!("../../data/monsters.json"));

    let height_sampler = Arc::new(create_height_sampler(
        &config.terrain,
        &config.terrain_cache,
    ));

    // NPC patrols keep loading tiles; sweep idle ones like the server does.
    let height_sampler_for_sweep = Arc::clone(&height_sampler);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(onlinerpg_terrain::TILE_CACHE_SWEEP_PERIOD).await;
            height_sampler_for_sweep.sweep_stale_tiles().await;
        }
    });

    // No hub when the panel is off, so sessions record nothing for it.
    let watch_hub = (config.watch_port != 0).then(|| {
        let hub = Arc::new(watch::WatchHub::new(
            npcs.iter().map(|n| n.label().to_string()).collect(),
        ));
        // Start before sign-in so the page is reachable while the device flow waits.
        tokio::spawn(watch::serve(
            Arc::clone(&hub),
            watch::MinimapSource::new(&config.terrain),
            config.watch_port,
        ));
        hub
    });

    let world_cache = {
        let mut cache = state::WorldCache::new();
        cache.register_dungeons();
        Arc::new(std::sync::RwLock::new(cache))
    };

    let shared = Arc::new(SharedResources {
        height_sampler,
        world_cache,
        behavior_trees: Arc::new(behavior_trees),
        type_mapping: Arc::new(type_mapping),
        movement_speeds: Arc::new(movement_speeds),
        scheduler: llm_scheduler::LlmScheduler::new(
            config.max_concurrent,
            Duration::from_secs(config.request_timeout_secs),
        ),
        auth: match config.auth.mode {
            AuthMode::NpcToken => AuthSource::NpcToken(resolve_npc_token(config.npc_token)?),
            AuthMode::Google => {
                AuthSource::Google(google_auth::GoogleAuth::sign_in(config.auth.google).await?)
            }
        },
        watch: watch_hub,
    });

    orchestrator::run_orchestrator(config.server, npcs, shared).await
}

/// Guard rails for `mode = "google"`: this client speaks for a person's own
/// account, so it must not impersonate a registry NPC or take a class the
/// game does not offer players (`doc/REMOTE_AGENT_CLIENT.md`).
fn check_google_mode_config(npcs: &[NpcConfig]) -> anyhow::Result<()> {
    for npc in npcs {
        if let Some(id) = &npc.id {
            anyhow::bail!(
                "[[npcs]] id = \"{id}\" is an operator NPC and cannot run under \
                 [auth] mode = \"google\"; give character_name/character_class instead"
            );
        }
        if let Some(class) = &npc.character_class {
            // Same rule the server enforces; checked here only so the mistake
            // surfaces at startup instead of at character creation.
            let parsed = class
                .parse::<onlinerpg_shared::CharacterClass>()
                .map_err(|_| anyhow::anyhow!("unknown character_class {class:?}"))?;
            if !parsed.is_player_selectable() {
                anyhow::bail!("character_class = \"{class}\" is not selectable by players");
            }
        }
        if npc.character_name.is_none() {
            anyhow::bail!("[auth] mode = \"google\" needs a character_name in every [[npcs]]");
        }
        if npc.account.is_some() {
            warn!(
                "Ignoring account = {:?}: the Google account decides who you are",
                npc.account
            );
        }
    }
    Ok(())
}

fn create_height_sampler(terrain: &str, cache_dir: &str) -> HeightSampler {
    if is_http_source(terrain) {
        info!("Heightmaps over HTTP from {terrain} (cache: {cache_dir})");
        return HeightSampler::new(terrain_http::HttpHeightTiles::new(
            terrain,
            std::path::PathBuf::from(cache_dir),
        ));
    }
    HeightSampler::new(TerrainIO::new(std::path::PathBuf::from(terrain)))
}

/// Fill an `[[npcs]]` entry from the game-data registry (`data-src/npcs.csv`,
/// the single source of truth for who an NPC is). `id` selects the registry
/// row; the character name and class come from it, and the prompt/schedule
/// files follow the `data/npcs/{id}/` directory convention. Explicit config
/// fields still win, so a deployment can override any of them. Entries
/// without `id` keep working fully spelled out (ad-hoc NPCs).
fn resolve_from_registry(npc: &mut NpcConfig) -> anyhow::Result<()> {
    let Some(id) = npc.id.clone() else {
        return Ok(());
    };
    let row = shop_info::npc_by_id(&id).ok_or_else(|| {
        anyhow::anyhow!("[[npcs]] id \"{id}\" is not in the NPC registry (data-src/npcs.csv)")
    })?;

    npc.character_name
        .get_or_insert_with(|| row.npc_name.clone());
    if npc.character_class.is_none() && !row.class.is_empty() {
        npc.character_class = Some(row.class.clone());
    }
    if npc.template_prompt.is_none() {
        let class = npc.character_class.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "registry NPC \"{id}\" has no class; add one in data-src/npcs.csv \
                 or set character_class/template_prompt in config.toml"
            )
        })?;
        npc.template_prompt = Some(format!("data/templates/{class}.txt"));
    }
    npc.instance_prompt
        .get_or_insert_with(|| format!("data/npcs/{id}/instance.txt"));
    npc.memory_file
        .get_or_insert_with(|| format!("data/npcs/{id}/memory.txt"));
    if npc.schedule_file.is_none() {
        // Schedules are optional, and a missing path is logged as an error
        // downstream — only derive it when the conventional file exists.
        let path = format!("data/npcs/{id}/schedule.json");
        if std::path::Path::new(&path).exists() {
            npc.schedule_file = Some(path);
        }
    }
    Ok(())
}

pub fn msg_name(msg: &onlinerpg_shared::ServerMessage) -> &'static str {
    use onlinerpg_shared::ServerMessage;
    match msg {
        ServerMessage::AuthSuccess { .. } => "AuthSuccess",
        ServerMessage::AuthError { .. } => "AuthError",
        ServerMessage::JoinSuccess { .. } => "JoinSuccess",
        ServerMessage::CharacterCreated { .. } => "CharacterCreated",
        ServerMessage::CharacterStatsRolled { .. } => "CharacterStatsRolled",
        ServerMessage::CharacterDeleted { .. } => "CharacterDeleted",
        ServerMessage::CharacterError { .. } => "CharacterError",
        ServerMessage::PlayerJoined { .. } => "PlayerJoined",
        ServerMessage::PlayerLeft { .. } => "PlayerLeft",
        ServerMessage::PlayerAppeared { .. } => "PlayerAppeared",
        ServerMessage::PlayerDisappeared { .. } => "PlayerDisappeared",
        ServerMessage::PlayerMoved { .. } => "PlayerMoved",
        ServerMessage::PlayerTeleported { .. } => "PlayerTeleported",
        ServerMessage::PositionCorrected { .. } => "PositionCorrected",
        ServerMessage::DungeonChestOpened { .. } => "DungeonChestOpened",
        ServerMessage::DungeonPropBroken { .. } => "DungeonPropBroken",
        ServerMessage::DungeonPropOpened { .. } => "DungeonPropOpened",
        ServerMessage::DungeonPropsState { .. } => "DungeonPropsState",
        ServerMessage::DungeonDoorToggled { .. } => "DungeonDoorToggled",
        ServerMessage::DungeonDoorsState { .. } => "DungeonDoorsState",
        ServerMessage::DungeonDiscoveries { .. } => "DungeonDiscoveries",
        ServerMessage::ChatMessage { .. } => "ChatMessage",
        ServerMessage::WhisperMessage { .. } => "WhisperMessage",
        ServerMessage::PartyChatMessage { .. } => "PartyChatMessage",
        ServerMessage::SystemMessage { .. } => "SystemMessage",
        ServerMessage::GameState { .. } => "GameState",
        ServerMessage::GameTimeSync { .. } => "GameTimeSync",
        ServerMessage::MonsterSpawned { .. } => "MonsterSpawned",
        ServerMessage::MonsterMoved { .. } => "MonsterMoved",
        ServerMessage::MonsterRemoved { .. } => "MonsterRemoved",
        ServerMessage::MonsterDead { .. } => "MonsterDead",
        ServerMessage::PlayerAttacked { .. } => "PlayerAttacked",
        ServerMessage::PlayerAttackRejected { .. } => "PlayerAttackRejected",
        ServerMessage::MonsterProvoked { .. } => "MonsterProvoked",
        ServerMessage::MonsterAttackedPlayer { .. } => "MonsterAttackedPlayer",
        ServerMessage::PlayerDead { .. } => "PlayerDead",
        ServerMessage::PlayerRespawned { .. } => "PlayerRespawned",
        ServerMessage::PlayerHealthUpdate { .. } => "PlayerHealthUpdate",
        ServerMessage::XpGained { .. } => "XpGained",
        ServerMessage::SkillsUpdate { .. } => "SkillsUpdate",
        ServerMessage::SkillXpGained { .. } => "SkillXpGained",
        ServerMessage::FishingCasted { .. } => "FishingCasted",
        ServerMessage::FishingBite { .. } => "FishingBite",
        ServerMessage::FishingFight { .. } => "FishingFight",
        ServerMessage::FishingEnded { .. } => "FishingEnded",
        ServerMessage::FishingError { .. } => "FishingError",
        ServerMessage::Kicked { .. } => "Kicked",
        ServerMessage::ServerNotice { .. } => "ServerNotice",
        ServerMessage::PlayerTorchToggled { .. } => "PlayerTorchToggled",
        ServerMessage::PlayerMainHandChanged { .. } => "PlayerMainHandChanged",
        ServerMessage::HouseSpawned { .. } => "HouseSpawned",
        ServerMessage::HouseUpdated { .. } => "HouseUpdated",
        ServerMessage::TreeTilesInvalidated { .. } => "TreeTilesInvalidated",
        ServerMessage::HouseRemoved { .. } => "HouseRemoved",
        ServerMessage::HousesInArea { .. } => "HousesInArea",
        ServerMessage::DoorToggled { .. } => "DoorToggled",
        ServerMessage::MonsterAssigned { .. } => "MonsterAssigned",
        ServerMessage::SpawnMonsterRequest { .. } => "SpawnMonsterRequest",
        ServerMessage::NoSpawnZones { .. } => "NoSpawnZones",
        ServerMessage::PlayerInteractionChanged { .. } => "PlayerInteractionChanged",
        ServerMessage::PlayerMusicStarted { .. } => "PlayerMusicStarted",
        ServerMessage::InteractionRejected { .. } => "InteractionRejected",
        ServerMessage::InventoryState { .. } => "InventoryState",
        ServerMessage::InventoryUpdated { .. } => "InventoryUpdated",
        ServerMessage::GroundItemSpawned { .. } => "GroundItemSpawned",
        ServerMessage::GroundItemAppeared { .. } => "GroundItemAppeared",
        ServerMessage::GroundItemRemoved { .. } => "GroundItemRemoved",
        ServerMessage::ShopState { .. } => "ShopState",
        ServerMessage::GoldUpdate { .. } => "GoldUpdate",
        ServerMessage::GuardUpdated { .. } => "GuardUpdated",
        ServerMessage::GoldGained { .. } => "GoldGained",
        ServerMessage::TradeError { .. } => "TradeError",
        ServerMessage::DealUpdated { .. } => "DealUpdated",
        ServerMessage::BuybackUpdated { .. } => "BuybackUpdated",
        ServerMessage::DealResult { .. } => "DealResult",
        ServerMessage::TradeNotice { .. } => "TradeNotice",
        ServerMessage::TradeBusy { .. } => "TradeBusy",
        ServerMessage::PartyInviteReceived { .. } => "PartyInviteReceived",
        ServerMessage::PartyInviteResult { .. } => "PartyInviteResult",
        ServerMessage::PartySummonReceived { .. } => "PartySummonReceived",
        ServerMessage::PartyState { .. } => "PartyState",
        ServerMessage::PartyPositions { .. } => "PartyPositions",
        ServerMessage::HungerUpdate { .. } => "HungerUpdate",
        ServerMessage::CampfireSpawned { .. } => "CampfireSpawned",
        ServerMessage::CampfireAppeared { .. } => "CampfireAppeared",
        ServerMessage::CampfireRemoved { .. } => "CampfireRemoved",
        ServerMessage::GrillStarted => "GrillStarted",
        ServerMessage::GrillEnded { .. } => "GrillEnded",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(config: &str) -> Config {
        toml::from_str(config).expect("config should parse")
    }

    const GOOGLE_BASE: &str = r#"
server = "wss://example.test/ws"
[auth]
mode = "google"
"#;

    #[test]
    fn max_concurrent_must_leave_at_least_one_scheduler_slot() {
        let zero = parse(
            r#"
server = "ws://127.0.0.1:10006"
max_concurrent = 0
"#,
        );
        let defaulted = parse("server = \"ws://127.0.0.1:10006\"\n");

        let err = check_max_concurrent(zero.max_concurrent)
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least 1"), "{err}");
        assert_eq!(defaulted.max_concurrent, 2);
        assert!(check_max_concurrent(defaulted.max_concurrent).is_ok());
        assert!(check_max_concurrent(1).is_ok());
    }

    /// Who pays decides the default: an agent someone runs for themselves
    /// spends their own LLM quota, so it keeps thinking with nobody watching.
    /// A registry NPC runs on the operator's budget and stays gated.
    #[test]
    fn only_registry_npcs_wait_for_an_audience() {
        let config = parse(
            r#"
server = "ws://127.0.0.1:10006"

[[npcs]]
id = "karl"
account = "npc_guard"

[[npcs]]
account = "npc_delver"
character_name = "Delver"

[[npcs]]
id = "rica"
account = "npc_merchant"
always_active = true
"#,
        );
        assert!(!config.npcs[0].always_active(), "registry NPC");
        assert!(config.npcs[1].always_active(), "player-run agent");
        assert!(config.npcs[2].always_active(), "explicit override wins");
    }

    /// Every registry NPC must find the prompt files the directory convention
    /// promises. A missing one is a startup crash on the server, in the dark.
    #[test]
    fn registry_npcs_resolve_to_prompt_files_that_exist() {
        for id in ["karl", "rica", "signe"] {
            let config = parse(&format!(
                "server = \"ws://127.0.0.1:10006\"\n\n[[npcs]]\nid = \"{id}\"\n"
            ));
            let mut npc = config.npcs.into_iter().next().expect("one npc");
            resolve_from_registry(&mut npc).expect("registry row");
            for path in [
                npc.template_prompt.expect("class template"),
                npc.instance_prompt.expect("instance prompt"),
            ] {
                assert!(
                    std::path::Path::new(&path).exists(),
                    "{id}: {path} is missing"
                );
            }
        }
    }

    #[test]
    fn auth_defaults_to_the_npc_token_flow() {
        let config = parse("server = \"ws://127.0.0.1:10006\"\n");
        assert_eq!(config.auth.mode, AuthMode::NpcToken);
        assert_eq!(config.auth.google.client_id, google_auth::DEFAULT_CLIENT_ID);
        assert_eq!(config.terrain, default_terrain());
    }

    #[test]
    fn google_auth_settings_sit_in_the_auth_table() {
        let config = parse(
            r#"
server = "wss://example.test/ws"

[auth]
mode = "google"
client_id = "custom.apps.googleusercontent.com"
token_cache = "/tmp/creds.json"
"#,
        );
        assert_eq!(config.auth.mode, AuthMode::Google);
        assert_eq!(
            config.auth.google.client_id,
            "custom.apps.googleusercontent.com"
        );
        assert_eq!(
            config.auth.google.token_cache.as_deref(),
            Some("/tmp/creds.json")
        );
    }

    #[test]
    fn terrain_dir_still_parses_as_an_alias() {
        let config = parse("server = \"ws://x\"\nterrain_dir = \"/data/terrain\"\n");
        assert_eq!(config.terrain, "/data/terrain");
        assert!(!is_http_source(&config.terrain));
        assert!(is_http_source("https://example.test"));
    }

    #[test]
    fn google_mode_rejects_registry_npcs() {
        let config = parse(&format!("{GOOGLE_BASE}\n[[npcs]]\nid = \"karl\"\n"));
        let err = check_google_mode_config(&config.npcs)
            .unwrap_err()
            .to_string();
        assert!(err.contains("karl"), "{err}");
    }

    #[test]
    fn google_mode_rejects_operator_only_classes() {
        for class in ["merchant", "guard"] {
            let config = parse(&format!(
                "{GOOGLE_BASE}\n[[npcs]]\ncharacter_name = \"A\"\ncharacter_class = \"{class}\"\n"
            ));
            assert!(check_google_mode_config(&config.npcs).is_err(), "{class}");
        }
    }

    #[test]
    fn google_mode_accepts_a_plain_player_agent() {
        let config = parse(&format!(
            "{GOOGLE_BASE}\n[[npcs]]\ncharacter_name = \"Jake's Agent\"\ncharacter_class = \"rogue\"\ngender = \"female\"\n"
        ));
        assert!(check_google_mode_config(&config.npcs).is_ok());
        assert_eq!(
            config.npcs[0].gender,
            Some(onlinerpg_shared::Gender::Female)
        );
    }

    #[test]
    fn google_mode_needs_a_character_name() {
        let config = parse(&format!(
            "{GOOGLE_BASE}\n[[npcs]]\ncharacter_class = \"ranger\"\n"
        ));
        assert!(check_google_mode_config(&config.npcs).is_err());
    }
}

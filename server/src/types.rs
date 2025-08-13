use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub position: Position,
    pub level: u32,
    pub health: u32,
    pub max_health: u32,
}

impl Player {
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            position: Position { x: 0.0, y: 0.0, z: 0.0 },
            level: 1,
            health: 100,
            max_health: 100,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "join")]
    Join { player_name: String },
    #[serde(rename = "player_move")]
    PlayerMove { position: Position },
    #[serde(rename = "chat_message")]
    ChatMessage { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "player_joined")]
    PlayerJoined { player: Player },
    #[serde(rename = "player_left")]
    PlayerLeft { player_id: String },
    #[serde(rename = "player_moved")]
    PlayerMoved { player_id: String, position: Position },
    #[serde(rename = "chat_message")]
    ChatMessage { player_name: String, message: String },
    #[serde(rename = "game_state")]
    GameState { players: HashMap<String, Player> },
}

pub type PlayerId = String;
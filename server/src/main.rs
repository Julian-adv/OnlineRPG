mod types;
mod game_state;
mod connection;

use connection::handle_connection;
use game_state::GameState;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let game_state = Arc::new(GameState::new());
    
    let addr = "127.0.0.1:8080";
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => {
            info!("MMORPG Server listening on: {}", addr);
            listener
        }
        Err(e) => {
            error!("Failed to bind to {}: {}", addr, e);
            return;
        }
    };

    info!("🎮 MMORPG Server started successfully!");
    info!("📡 WebSocket server ready for connections");
    info!("🌐 Connect clients to: ws://{}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!("New connection from: {}", addr);
                let game_state_clone = Arc::clone(&game_state);
                
                tokio::spawn(async move {
                    handle_connection(stream, game_state_clone).await;
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}

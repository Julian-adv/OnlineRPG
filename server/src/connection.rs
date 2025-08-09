use crate::game_state::GameState;
use crate::types::{ClientMessage, Player, PlayerId};
use futures_util::{SinkExt, StreamExt};
use serde_json;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};

pub async fn handle_connection(stream: TcpStream, game_state: Arc<GameState>) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            error!("WebSocket handshake failed: {}", e);
            return;
        }
    };

    info!("New WebSocket connection established");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let mut game_receiver = game_state.subscribe();
    let mut player_id: Option<PlayerId> = None;

    loop {
        tokio::select! {
            // Handle incoming messages from client
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        info!("Received message: {}", text);
                        if let Err(e) = handle_client_message(
                            &text, 
                            &game_state, 
                            &mut player_id
                        ).await {
                            error!("Error handling client message: {} - message was: {}", e, text);
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Client requested close");
                        break;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        info!("WebSocket stream ended");
                        break;
                    }
                    _ => {}
                }
            }

            // Handle game state broadcasts
            broadcast_msg = game_receiver.recv() => {
                match broadcast_msg {
                    Ok(server_msg) => {
                        if let Ok(json) = serde_json::to_string(&server_msg) {
                            if let Err(e) = ws_sender.send(Message::Text(json)).await {
                                error!("Failed to send message to client: {}", e);
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Game state broadcast channel closed");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("Client lagged behind, skipped {} messages", skipped);
                    }
                }
            }
        }
    }

    // Clean up on disconnect
    if let Some(id) = player_id {
        game_state.remove_player(&id).await;
    }
    
    info!("Connection handler finished");
}

async fn handle_client_message(
    message: &str,
    game_state: &Arc<GameState>,
    player_id: &mut Option<PlayerId>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client_msg: ClientMessage = serde_json::from_str(message)?;

    match client_msg {
        ClientMessage::Join { player_name } => {
            if player_id.is_some() {
                warn!("Player already joined, ignoring join request");
                return Ok(());
            }

            let player = Player::new(player_name);
            let id = player.id.clone();
            
            game_state.add_player(player).await;
            *player_id = Some(id);
            
            info!("Player joined with ID: {:?}", player_id);
        }

        ClientMessage::PlayerMove { position } => {
            if let Some(id) = player_id {
                game_state.update_player_position(id, position).await;
            } else {
                warn!("Received move from unauthenticated client");
            }
        }

        ClientMessage::ChatMessage { message } => {
            if let Some(id) = player_id {
                game_state.send_chat_message(id, message).await;
            } else {
                warn!("Received chat message from unauthenticated client");
            }
        }
    }

    Ok(())
}
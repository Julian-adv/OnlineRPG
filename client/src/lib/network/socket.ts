import { gameStore, updatePlayer, addChatMessage } from '../stores/gameStore';
import type { Player } from '../stores/gameStore';
import { Vector3 } from 'three';

type Position = {
	x: number;
	y: number;
	z: number;
};

type ServerPlayer = {
	id: string;
	name: string;
	position: Position;
	level: number;
	health: number;
	max_health: number;
};

type ClientMessage = 
	| { type: 'join'; player_name: string }
	| { type: 'player_move'; position: Position }
	| { type: 'chat_message'; message: string };

type ServerMessage = 
	| { type: 'player_joined'; player: ServerPlayer }
	| { type: 'player_left'; player_id: string }
	| { type: 'player_moved'; player_id: string; position: Position }
	| { type: 'chat_message'; player_name: string; message: string }
	| { type: 'game_state'; players: Record<string, ServerPlayer> };

class NetworkManager {
	private socket: WebSocket | null = null;
	private reconnectAttempts = 0;
	private maxReconnectAttempts = 5;
	private reconnectTimer: ReturnType<typeof setTimeout> | null = null;

	connect(serverUrl: string = 'ws://localhost:8080') {
		if (this.socket?.readyState === WebSocket.OPEN) {
			console.log('Already connected, skipping connection attempt');
			return;
		}
		
		if (this.socket?.readyState === WebSocket.CONNECTING) {
			console.log('Connection in progress, skipping connection attempt');
			return;
		}

		console.log('Attempting to connect to:', serverUrl);
		this.socket = new WebSocket(serverUrl);

		this.socket.onopen = () => {
			console.log('Connected to server');
			gameStore.update(state => ({ ...state, isConnected: true }));
			this.reconnectAttempts = 0;
			if (this.reconnectTimer) {
				clearTimeout(this.reconnectTimer);
				this.reconnectTimer = null;
			}
		};

		this.socket.onclose = (event) => {
			console.log('Disconnected from server', event.code, event.reason);
			gameStore.update(state => ({ ...state, isConnected: false }));
			
			// Only reconnect if it wasn't a manual disconnect
			if (event.code !== 1000) {
				this.handleReconnect();
			}
		};

		this.socket.onerror = (error) => {
			console.error('WebSocket error:', error);
			this.handleReconnect();
		};

		this.socket.onmessage = (event) => {
			try {
				const message: ServerMessage = JSON.parse(event.data);
				this.handleServerMessage(message);
			} catch (error) {
				console.error('Error parsing server message:', error);
			}
		};
	}

	private handleReconnect() {
		if (this.reconnectAttempts < this.maxReconnectAttempts && !this.reconnectTimer) {
			this.reconnectAttempts++;
			console.log(`Reconnection attempt ${this.reconnectAttempts}/${this.maxReconnectAttempts}`);
			this.reconnectTimer = setTimeout(() => {
				this.reconnectTimer = null;
				this.connect();
			}, 2000 * this.reconnectAttempts);
		}
	}

	private handleServerMessage(message: ServerMessage) {
		switch (message.type) {
			case 'player_joined':
				const playerPosition = new Vector3(message.player.position.x, message.player.position.y, message.player.position.z);
				const player: Player = {
					...message.player,
					position: playerPosition,
					maxHealth: message.player.max_health
				};
				gameStore.update(state => {
					state.otherPlayers.set(message.player.id, player);
					return state;
				});
				addChatMessage(`${message.player.name} joined the game`);
				break;

			case 'player_left':
				gameStore.update(state => {
					const player = state.otherPlayers.get(message.player_id);
					if (player) {
						state.otherPlayers.delete(message.player_id);
						addChatMessage(`${player.name} left the game`);
					}
					return state;
				});
				break;

			case 'player_moved':
				const position = new Vector3(message.position.x, message.position.y, message.position.z);
				updatePlayer(message.player_id, { position });
				break;

			case 'chat_message':
				addChatMessage(`${message.player_name}: ${message.message}`);
				break;

			case 'game_state':
				gameStore.update(state => {
					state.otherPlayers.clear();
					Object.values(message.players).forEach(serverPlayer => {
						if (serverPlayer.id !== state.currentPlayer?.id) {
							const playerPos = new Vector3(serverPlayer.position.x, serverPlayer.position.y, serverPlayer.position.z);
							const player: Player = {
								...serverPlayer,
								position: playerPos,
								maxHealth: serverPlayer.max_health
							};
							state.otherPlayers.set(serverPlayer.id, player);
						}
					});
					return state;
				});
				break;
		}
	}

	sendPlayerMove(position: { x: number, y: number, z: number }) {
		if (this.socket?.readyState === WebSocket.OPEN) {
			const message: ClientMessage = {
				type: 'player_move',
				position
			};
			this.socket.send(JSON.stringify(message));
		}
	}

	sendChatMessage(message: string) {
		if (this.socket?.readyState === WebSocket.OPEN) {
			const clientMessage: ClientMessage = {
				type: 'chat_message',
				message
			};
			this.socket.send(JSON.stringify(clientMessage));
		}
	}

	joinGame(playerName: string) {
		if (this.socket?.readyState === WebSocket.OPEN) {
			const message: ClientMessage = {
				type: 'join',
				player_name: playerName
			};
			this.socket.send(JSON.stringify(message));
			gameStore.update(state => ({
				...state,
				currentPlayer: {
					id: '',
					name: playerName,
					position: new Vector3(0, 1, 0),
					level: 1,
					health: 100,
					maxHealth: 100
				}
			}));
		}
	}

	disconnect() {
		if (this.reconnectTimer) {
			clearTimeout(this.reconnectTimer);
			this.reconnectTimer = null;
		}
		if (this.socket) {
			this.socket.close();
			this.socket = null;
		}
	}
}

export const networkManager = new NetworkManager();
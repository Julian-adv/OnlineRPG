import { io, type Socket } from 'socket.io-client';
import { gameStore, updatePlayer, addChatMessage } from '../stores/gameStore';
import type { Player } from '../stores/gameStore';

class NetworkManager {
	private socket: Socket | null = null;
	private reconnectAttempts = 0;
	private maxReconnectAttempts = 5;

	connect(serverUrl: string = 'ws://localhost:8080') {
		if (this.socket?.connected) return;

		this.socket = io(serverUrl, {
			transports: ['websocket']
		});

		this.socket.on('connect', () => {
			console.log('Connected to server');
			gameStore.update(state => ({ ...state, isConnected: true }));
			this.reconnectAttempts = 0;
		});

		this.socket.on('disconnect', () => {
			console.log('Disconnected from server');
			gameStore.update(state => ({ ...state, isConnected: false }));
		});

		this.socket.on('player_joined', (player: Player) => {
			gameStore.update(state => {
				state.otherPlayers.set(player.id, player);
				return state;
			});
			addChatMessage(`${player.name} joined the game`);
		});

		this.socket.on('player_left', (playerId: string) => {
			gameStore.update(state => {
				const player = state.otherPlayers.get(playerId);
				if (player) {
					state.otherPlayers.delete(playerId);
					addChatMessage(`${player.name} left the game`);
				}
				return state;
			});
		});

		this.socket.on('player_moved', (data: { playerId: string, position: any }) => {
			updatePlayer(data.playerId, { position: data.position });
		});

		this.socket.on('chat_message', (data: { playerName: string, message: string }) => {
			addChatMessage(`${data.playerName}: ${data.message}`);
		});

		this.socket.on('connect_error', () => {
			this.handleReconnect();
		});
	}

	private handleReconnect() {
		if (this.reconnectAttempts < this.maxReconnectAttempts) {
			this.reconnectAttempts++;
			console.log(`Reconnection attempt ${this.reconnectAttempts}/${this.maxReconnectAttempts}`);
			setTimeout(() => this.connect(), 2000 * this.reconnectAttempts);
		}
	}

	sendPlayerMove(position: { x: number, y: number, z: number }) {
		if (this.socket?.connected) {
			this.socket.emit('player_move', position);
		}
	}

	sendChatMessage(message: string) {
		if (this.socket?.connected) {
			this.socket.emit('chat_message', message);
		}
	}

	disconnect() {
		if (this.socket) {
			this.socket.disconnect();
			this.socket = null;
		}
	}
}

export const networkManager = new NetworkManager();
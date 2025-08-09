import { writable } from 'svelte/store';
import type { Vector3 } from 'three';

export interface Player {
	id: string;
	name: string;
	position: Vector3;
	level: number;
	health: number;
	maxHealth: number;
}

export interface GameState {
	isConnected: boolean;
	currentPlayer: Player | null;
	otherPlayers: Map<string, Player>;
	chatMessages: string[];
}

const initialGameState: GameState = {
	isConnected: false,
	currentPlayer: null,
	otherPlayers: new Map(),
	chatMessages: []
};

export const gameStore = writable<GameState>(initialGameState);

export const updatePlayer = (playerId: string, playerData: Partial<Player>) => {
	gameStore.update(state => {
		if (state.currentPlayer && state.currentPlayer.id === playerId) {
			state.currentPlayer = { ...state.currentPlayer, ...playerData };
		} else {
			const existingPlayer = state.otherPlayers.get(playerId);
			if (existingPlayer) {
				state.otherPlayers.set(playerId, { ...existingPlayer, ...playerData });
			}
		}
		return state;
	});
};

export const addChatMessage = (message: string) => {
	gameStore.update(state => {
		state.chatMessages.push(message);
		if (state.chatMessages.length > 100) {
			state.chatMessages = state.chatMessages.slice(-100);
		}
		return state;
	});
};
import { writable } from 'svelte/store'
import { SvelteMap } from 'svelte/reactivity'
import type { Vector3 } from 'three'
import type { CharacterClass, Gender } from '../network/networkTypes'
import { resetInventoryStore } from './inventoryStore'
import { resetSkillsStore } from './skillsStore'
import { groundItemManager } from '../managers/groundItemManager'

export interface PlayerDamageInfo {
  damage: number
  hit: boolean
  trigger: number
  currentHealth?: number
}

export interface PlayerGoldInfo {
  amount: number
  trigger: number
}

interface PlayerBase {
  id: number
  name: string
  level: number
  totalXp?: number
  health: number
  maxHealth: number
  characterClass: CharacterClass
  gender: Gender
  torchOn?: boolean
  lastDamageInfo?: PlayerDamageInfo
  lastRegenInfo?: PlayerDamageInfo
  lastGoldInfo?: PlayerGoldInfo
}

export interface LocalPlayer extends PlayerBase {
  position: Vector3
  rotation: number
}

export interface RemotePlayer extends PlayerBase {
  floorLevel: number
  isOfficialNpc: boolean
}

export interface ChatBubble {
  playerId: number
  message: string
  timestamp: number
  duration: number
}

export type ChatSender = 'local' | 'remote' | 'system' | 'whisper'

export interface ChatEntry {
  text: string
  sender: ChatSender
  name?: string
  hit?: boolean
}

/** ChatEntry as stored: `id` is a stable key so the transcript's `{#each}`
 *  moves rows instead of rewriting every one when the buffer shifts. */
export interface StoredChatEntry extends ChatEntry {
  id: number
}

export interface GameState {
  isConnected: boolean
  currentPlayer: LocalPlayer | null
  otherPlayers: Map<number, RemotePlayer>
  chatMessages: StoredChatEntry[]
  combatMessages: StoredChatEntry[]
  chatBubbles: Map<number, ChatBubble> // playerId -> ChatBubble
}

const initialGameState: GameState = {
  isConnected: false,
  currentPlayer: null,
  otherPlayers: new SvelteMap(),
  chatMessages: [],
  combatMessages: [],
  chatBubbles: new Map(),
}

export const gameStore = writable<GameState>(initialGameState)

/** World-space anchor + text of the placed object (e.g. signpost) currently
 *  under the cursor, or null when none. Drives the hover speech bubble. */
export interface HoveredSignpost {
  x: number
  y: number
  z: number
  text: string
}
export const hoveredSignpost = writable<HoveredSignpost | null>(null)

/** Set from JoinSuccess; unlocks debug/cheat UI (server re-validates). */
export const isAdminUser = writable(false)

/** Set from ServerNotice; cleared when the socket reopens. Kept out of
 *  `gameStore` so the HUD banner doesn't resubscribe on every game update. */
export const serverNotice = writable<string | null>(null)

export const resetGameStore = () => {
  gameStore.set({
    ...initialGameState,
    otherPlayers: new SvelteMap(),
    chatBubbles: new Map(),
  })
  isAdminUser.set(false)
  resetInventoryStore()
  resetSkillsStore()
  groundItemManager.reset()
}

const MAX_MESSAGES = 100

export const updatePlayer = (
  playerId: number,
  playerData: Partial<LocalPlayer> | Partial<RemotePlayer>
) => {
  gameStore.update((state) => {
    if (state.currentPlayer && state.currentPlayer.id === playerId) {
      return {
        ...state,
        currentPlayer: { ...state.currentPlayer, ...playerData },
      }
    } else {
      const existingPlayer = state.otherPlayers.get(playerId)
      if (existingPlayer) {
        state.otherPlayers.set(playerId, { ...existingPlayer, ...playerData })
      }
    }
    return state
  })
}

let nextMessageId = 0

const addMessageTo = (
  field: 'chatMessages' | 'combatMessages',
  entry: ChatEntry
) => {
  gameStore.update((state) => {
    const newMessages = [...state[field], { ...entry, id: nextMessageId++ }]
    return {
      ...state,
      [field]:
        newMessages.length > MAX_MESSAGES
          ? newMessages.slice(-MAX_MESSAGES)
          : newMessages,
    }
  })
}

export const addChatMessage = (entry: ChatEntry) =>
  addMessageTo('chatMessages', entry)

export const addCombatMessage = (entry: ChatEntry) =>
  addMessageTo('combatMessages', entry)

const MIN_BUBBLE_DURATION = 5000
const MAX_BUBBLE_DURATION = 10000

export const addChatBubble = (playerId: number, message: string) => {
  gameStore.update((state) => {
    const newChatBubbles = new Map(state.chatBubbles)
    const duration = Math.min(
      MAX_BUBBLE_DURATION,
      Math.max(MIN_BUBBLE_DURATION, MIN_BUBBLE_DURATION + message.length * 50)
    )
    newChatBubbles.set(playerId, {
      playerId,
      message,
      timestamp: Date.now(),
      duration,
    })
    return { ...state, chatBubbles: newChatBubbles }
  })
}

export const removeChatBubble = (playerId: number) => {
  gameStore.update((state) => {
    const newChatBubbles = new Map(state.chatBubbles)
    newChatBubbles.delete(playerId)
    return { ...state, chatBubbles: newChatBubbles }
  })
}

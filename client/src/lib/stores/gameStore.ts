import { derived, get, writable } from 'svelte/store'
import { SvelteMap } from 'svelte/reactivity'
import type { Vector3 } from 'three'
import type { CharacterClass, Gender } from '../network/networkTypes'
import type { HoverTarget } from '../managers/inputHandler'
import { resetInventoryStore } from './inventoryStore'
import { resetSkillsStore } from './skillsStore'
import { resetPartyStores } from './partyStore'
import { resetFriendStores } from './friendStore'
import { resetFishingStore } from './fishingStore'
import { resetDiscoveredDungeons } from './dungeonStore'
import { resetHungerStore } from './hungerStore'
import { resetDebuffStore } from './debuffStore'
import { resetHousingStore } from './housingStore'
import { groundItemManager } from '../managers/groundItemManager'
import { campfireManager } from '../managers/campfireManager'
import { stallManager } from '../managers/stallManager'
import { refreshBardZone } from '../managers/bardZone'

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
  /** Soaked, so nearby clients draw wet footprints (doc/DEBUFF.md). */
  wet?: boolean
  /** Shown title id (doc/TITLES.md). */
  title?: string | null
  mainHand?: string | null
  back?: string | null
  /** Dye on that cape, as broadcast with it. */
  backColor?: string | null
  backTexture?: string | null
  lastDamageInfo?: PlayerDamageInfo
  lastRegenInfo?: PlayerDamageInfo
  lastGoldInfo?: PlayerGoldInfo
}

export interface LocalPlayer extends PlayerBase {
  position: Vector3
  rotation: number
  /** Bumped on each blow that lands, to fire the flinch reaction. Remotes
   *  keep theirs in remotePlayerManager.hitCounters. */
  hitCounter?: number
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

export type ChatSender = 'local' | 'remote' | 'system' | 'whisper' | 'party'

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

/** What the cursor is over (texted object, ground item or monster), or null.
 *  Single source of truth: each hover overlay reads the variant it renders,
 *  so no two can be showing at once. */
export const hoverTarget = writable<HoverTarget | null>(null)

/** Placed object (e.g. signpost) under the cursor. Drives the speech bubble. */
export const hoveredSignpost = derived(hoverTarget, (target) =>
  target?.kind === 'text' ? target : null
)

/** Drop a 'name' hover: its target is a positional snapshot, so when the
 *  entity vanishes under a resting cursor (picked-up tip hat, closed stall)
 *  the label and ring would freeze in place until the next pointermove. */
export function clearNameHover() {
  if (get(hoverTarget)?.kind === 'name') hoverTarget.set(null)
}

/** Interactable prop under the cursor (tip hat, stall, chest). Drives the
 *  name label and target ring. */
export const hoveredNameLabel = derived(hoverTarget, (target) =>
  target?.kind === 'name' ? target : null
)

/** Ground item under the cursor. Every ground item subscribes, so this is
 *  derived to a plain id — a number the store dedupes, rather than an object
 *  that would wake all of them on any hover change. */
export const hoveredGroundItemId = derived(hoverTarget, (target) =>
  target?.kind === 'groundItem' ? target.instanceId : null
)

/** Monster under the cursor, as a deduped plain id for the same reason. */
export const hoveredMonsterId = derived(hoverTarget, (target) =>
  target?.kind === 'monster' ? target.monsterId : null
)

/** Remote player (NPC or not) under the cursor, deduped like the above. */
export const hoveredPlayerId = derived(hoverTarget, (target) =>
  target?.kind === 'player' ? target.playerId : null
)

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
  refreshBardZone(new Map())
  isAdminUser.set(false)
  resetInventoryStore()
  resetSkillsStore()
  resetFishingStore()
  resetPartyStores()
  resetFriendStores()
  resetDiscoveredDungeons()
  resetHungerStore()
  resetDebuffStore()
  resetHousingStore()
  groundItemManager.reset()
  campfireManager.reset()
  stallManager.reset()
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

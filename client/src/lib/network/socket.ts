import type { Position, PositionCorrection } from './networkTypes'
import { hmrSingleton } from '../utils/hmr'
import type { MonsterData } from '../types/Monster'
import type { WallDirection } from '../utils/house-geometry'
import { gameStore, resetGameStore, serverNotice } from '../stores/gameStore'
import { remotePlayerManager } from '../managers/remotePlayerManager'
import { monsterManager } from '../managers/monsterManager'
import {
  getApiAuthToken,
  getDefaultServerUrl,
  setApiAuthToken,
} from '../utils/networkUtils'
import { clearServerGameTime } from '../stores/timeStore'
import { markShopRequested, shopSession } from '../stores/tradeStore'
import initWasm, {
  serialize_client_message,
  deserialize_server_message,
  protocol_version,
  close_code_protocol_mismatch,
} from '../wasm/onlinerpg_shared'
import { createEvent } from './networkEvents'
import { handleServerMessage } from './messageHandlers'
import type {
  AccountCharacter,
  CharacterClass,
  CharacterRollResult,
  ClientMessage,
  EquipSlot,
  Gender,
  RollCharacterStatsResult,
} from './networkTypes'

export type {
  AccountCharacter,
  CharacterAttributes,
  CharacterClass,
  CharacterRollResult,
  Gender,
  RollCharacterStatsResult,
} from './networkTypes'

// wasm-bindgen copies the serialized bytes into a fresh, exactly-sized
// Uint8Array backed by a plain (non-shared) ArrayBuffer; its generated .d.ts
// just types it as Uint8Array<ArrayBufferLike>, which newer lib.dom versions
// reject for WebSocket.send. Narrow the type once at the wasm boundary.
function serializeClientMessage(msg: ClientMessage): Uint8Array<ArrayBuffer> {
  return serialize_client_message(msg) as Uint8Array<ArrayBuffer>
}

/// Reconnect pacing: exponential with full jitter under a hard ceiling, so a
/// server restart doesn't bring every client back as one synchronized wave.
const RECONNECT_BASE_DELAY_MS = 1000
const RECONNECT_MAX_DELAY_MS = 30_000
const MAX_RECONNECT_ATTEMPTS = 10

/// Cached from wasm once it loads. `onclose` also fires before that (a failed
/// TCP connect closes with 1006), so it stays null until then — and a refusal
/// can only reach a client that already sent a wasm-serialized message.
let protocolMismatchCloseCode: number | null = null

class NetworkManager {
  private socket: WebSocket | null = null
  private reconnectAttempts = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private lastServerUrl: string = ''
  private lastCharacterId: number | null = null
  private wasmReady = false
  /// Reset per socket: the handshake is per connection, not per session.
  private handshakeSent = false
  /// Server refused this build at the handshake. Reconnecting cannot fix a
  /// stale bundle, so every path stops trying until the page is reloaded.
  private refusedPermanently = false
  /// Reset per socket, like `handshakeSent`: the reason belongs to the
  /// connection that was refused, not to the session.
  private lastAuthErrorMessage: string | null = null

  // Events
  readonly respawnRequested = createEvent<() => void>()
  readonly playerRespawned = createEvent<(playerId: number) => void>()
  readonly authSuccess =
    createEvent<
      (payload: { accountName: string; characters: AccountCharacter[] }) => void
    >()
  readonly authError = createEvent<(message: string) => void>()
  readonly joinSuccess = createEvent<() => void>()
  readonly characterCreated =
    createEvent<(character: AccountCharacter) => void>()
  readonly characterStatsRolled =
    createEvent<(result: CharacterRollResult) => void>()
  readonly characterDeleted = createEvent<(characterId: number) => void>()
  readonly characterError = createEvent<(message: string) => void>()
  readonly kicked = createEvent<(reason: string) => void>()
  readonly interactionRejected = createEvent<(reason: string) => void>()
  readonly positionCorrected = createEvent<(c: PositionCorrection) => void>()

  constructor() {
    // Only a fully authenticated connection clears the counter; a socket that
    // merely opened proves nothing, since refusals arrive after the open.
    this.authSuccess.on(() => {
      this.reconnectAttempts = 0
    })
    this.authError.on((message) => {
      this.lastAuthErrorMessage = message
    })
  }

  private get messageEvents() {
    return {
      authSuccess: this.authSuccess,
      authError: this.authError,
      joinSuccess: this.joinSuccess,
      characterCreated: this.characterCreated,
      characterStatsRolled: this.characterStatsRolled,
      characterDeleted: this.characterDeleted,
      characterError: this.characterError,
      kicked: this.kicked,
      playerRespawned: this.playerRespawned,
      interactionRejected: this.interactionRejected,
      positionCorrected: this.positionCorrected,
    }
  }

  async ensureWasm() {
    if (!this.wasmReady) {
      await initWasm()
      protocolMismatchCloseCode = close_code_protocol_mismatch()
      this.wasmReady = true
    }
  }

  connect(serverUrl?: string) {
    if (this.refusedPermanently) {
      console.warn('Not connecting: the server refused this client build')
      return
    }

    if (serverUrl) {
      this.lastServerUrl = serverUrl
    } else if (!this.lastServerUrl) {
      this.lastServerUrl = getDefaultServerUrl()
    }

    const targetUrl = this.lastServerUrl

    if (this.socket?.readyState === WebSocket.OPEN) {
      console.log('Already connected, skipping connection attempt')
      return
    }

    if (this.socket?.readyState === WebSocket.CONNECTING) {
      console.log('Connection in progress, skipping connection attempt')
      return
    }

    console.log('Attempting to connect to:', targetUrl)
    this.handshakeSent = false
    this.lastAuthErrorMessage = null
    this.socket = new WebSocket(targetUrl)
    this.socket.binaryType = 'arraybuffer'

    this.socket.onopen = () => {
      console.log('Connected to server')
      // Mandatory first message. Sending it here covers the idle case; the
      // send path calls ensureHandshake() too, so a first message that races
      // this callback still goes out second.
      void this.ensureWasm().then(() => {
        if (this.socket?.readyState === WebSocket.OPEN) this.ensureHandshake()
      })
      gameStore.update((state) => ({ ...state, isConnected: true }))
      serverNotice.set(null)
      clearServerGameTime()
      if (this.reconnectTimer) {
        clearTimeout(this.reconnectTimer)
        this.reconnectTimer = null
      }
    }

    this.socket.onclose = (event) => {
      console.log('Disconnected from server', event.code, event.reason)
      gameStore.update((state) => ({ ...state, isConnected: false }))

      // The refusal's own AuthError carries the full "how to fix it" hint;
      // the close frame only has room for a short reason.
      if (event.code === protocolMismatchCloseCode) {
        this.refusedPermanently = true
        if (this.reconnectTimer) {
          clearTimeout(this.reconnectTimer)
          this.reconnectTimer = null
        }
        serverNotice.set(
          this.lastAuthErrorMessage ??
            'This client is out of date. Please reload the page.'
        )
        return
      }

      if (event.code !== 1000) {
        this.handleReconnect()
      }
    }

    this.socket.onerror = (error) => {
      console.error('WebSocket error:', error)
      this.handleReconnect()
    }

    this.socket.onmessage = (event) => {
      try {
        const bytes = new Uint8Array(event.data as ArrayBuffer)
        const message = deserialize_server_message(bytes)
        handleServerMessage(message, this.messageEvents, () =>
          this.disconnect()
        )
        // Respond to time sync with heartbeat so the server knows we're alive
        if (
          message &&
          typeof message === 'object' &&
          'GameTimeSync' in message
        ) {
          this.sendMessage('Heartbeat')
        }
      } catch (error) {
        console.error('Error deserializing server message:', error)
      }
    }
  }

  /// Full jitter: half the capped delay plus a random half, so clients that
  /// dropped together don't come back together.
  private reconnectDelay(): number {
    const capped = Math.min(
      RECONNECT_MAX_DELAY_MS,
      RECONNECT_BASE_DELAY_MS * 2 ** (this.reconnectAttempts - 1)
    )
    return Math.round(capped / 2 + Math.random() * (capped / 2))
  }

  private handleReconnect() {
    if (this.refusedPermanently || this.reconnectTimer) return

    if (this.reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
      serverNotice.set('Lost connection to the server. Please reload the page.')
      return
    }

    this.reconnectAttempts++
    const delay = this.reconnectDelay()
    console.log(
      `Reconnection attempt ${this.reconnectAttempts}/${MAX_RECONNECT_ATTEMPTS} in ${delay}ms`
    )
    this.reconnectTimer = setTimeout(async () => {
      this.reconnectTimer = null
      monsterManager.reset()
      remotePlayerManager.reset()
      this.connect()
      const googleIdToken = getApiAuthToken()
      if (googleIdToken && this.lastCharacterId) {
        const opened = await this.waitForSocketOpen(5000)
        if (opened) {
          this.authenticateWithGoogle(googleIdToken)
          let unsubSuccess = () => {}
          let unsubError = () => {}
          const cleanup = () => {
            unsubSuccess()
            unsubError()
          }
          unsubSuccess = this.authSuccess.on(() => {
            cleanup()
            if (this.lastCharacterId) {
              this.sendAndSerialize({
                EnterGame: { character_id: this.lastCharacterId },
              })
            }
          })
          // A cached Google ID token expires ~1h after login, so a reconnect
          // past that point fails re-auth. Surface it instead of leaving the
          // player silently stuck on an authenticated-but-empty socket.
          unsubError = this.authError.on((message) => {
            cleanup()
            console.warn('Reconnect auth failed:', message)
            this.disconnect()
            this.kicked.emit('Your session expired. Please sign in again.')
          })
        }
      }
    }, delay)
  }

  private sendMessage(msg: ClientMessage) {
    if (this.socket?.readyState === WebSocket.OPEN && this.wasmReady) {
      this.ensureHandshake()
      const bytes = serializeClientMessage(msg)
      this.socket.send(bytes)
    }
  }

  private isConnected(): boolean {
    return this.socket?.readyState === WebSocket.OPEN && this.wasmReady
  }

  private sendAndSerialize(msg: ClientMessage): boolean {
    if (!this.isConnected()) return false
    this.ensureHandshake()
    const bytes = serializeClientMessage(msg)
    this.socket!.send(bytes)
    return true
  }

  private requestWithTimeout<T>(
    timeoutMs: number,
    timeoutMessage: string,
    setup: (
      settle: (result: T) => void,
      onCleanup: (unsub: () => void) => void
    ) => {
      send: () => boolean
      notSentResult: T
    }
  ): Promise<T> {
    return new Promise((resolve) => {
      let settled = false
      const cleanups: (() => void)[] = []

      const settle = (result: T) => {
        if (settled) return
        settled = true
        clearTimeout(timeout)
        cleanups.forEach((fn) => fn())
        resolve(result)
      }

      const onCleanup = (unsub: () => void) => cleanups.push(unsub)

      const timeout = setTimeout(
        () => settle({ ok: false, message: timeoutMessage } as T),
        timeoutMs
      )

      const { send, notSentResult } = setup(settle, onCleanup)

      const sent = send()
      if (!sent && !settled) {
        settle(notSentResult)
      }
    })
  }

  private waitForSocketOpen(timeoutMs: number): Promise<boolean> {
    if (this.socket?.readyState === WebSocket.OPEN) {
      return Promise.resolve(true)
    }

    return new Promise((resolve) => {
      const start = Date.now()
      const interval = setInterval(() => {
        const socket = this.socket
        if (socket?.readyState === WebSocket.OPEN) {
          clearInterval(interval)
          resolve(true)
          return
        }

        const isClosed =
          !socket ||
          socket.readyState === WebSocket.CLOSING ||
          socket.readyState === WebSocket.CLOSED
        if (isClosed || Date.now() - start >= timeoutMs) {
          clearInterval(interval)
          resolve(false)
        }
      }, 50)
    })
  }

  // --- Public send methods ---

  sendPlayerAttack(monsterId: string) {
    this.sendMessage({ PlayerAttack: { monster_id: monsterId } })
  }

  sendMonsterAttack(monsterId: string, targetPlayerId: number) {
    this.sendMessage({
      MonsterAttack: {
        monster_id: monsterId,
        target_player_id: targetPlayerId,
      },
    })
  }

  requestRespawn() {
    if (this.sendAndSerialize('RequestRespawn')) {
      this.respawnRequested.emit()
    }
  }

  sendPlayerMove(
    position: { x: number; y: number; z: number },
    rotation: number,
    floorLevel: number,
    append = false
  ) {
    this.sendMessage({
      PlayerMove: { position, rotation, floor_level: floorLevel, append },
    })
  }

  /** Floor change between waypoints — see ClientMessage::PlayerFloorChanged. */
  sendPlayerFloor(floorLevel: number) {
    this.sendMessage({ PlayerFloorChanged: { floor_level: floorLevel } })
  }

  sendMonsterMove(
    monsterId: string,
    position: { x: number; y: number; z: number },
    rotation: number,
    state: MonsterData['state'],
    targetPosition: { x: number; y: number; z: number }
  ) {
    this.sendMessage({
      MonsterMove: {
        monster_id: monsterId,
        position,
        rotation,
        state,
        target_position: targetPosition,
      },
    })
  }

  sendDebugTeleport(position: Position) {
    this.sendMessage({ DebugTeleport: { position } })
  }

  sendOpenDungeonChest(entranceId: string) {
    this.sendMessage({ OpenDungeonChest: { entrance_id: entranceId } })
  }

  sendFishingCast(position: Position) {
    this.sendMessage({ FishingCast: { position } })
  }

  sendFishingRespond(action: import('./networkTypes').FishingAction) {
    this.sendMessage({ FishingRespond: { action } })
  }

  sendFishingStop() {
    this.sendMessage('FishingStop')
  }

  sendBreakDungeonProp(entranceId: string, depth: number, propId: number) {
    this.sendMessage({
      BreakDungeonProp: {
        entrance_id: entranceId,
        depth,
        prop_id: propId,
      },
    })
  }

  sendOpenDungeonProp(entranceId: string, depth: number, propId: number) {
    this.sendMessage({
      OpenDungeonProp: {
        entrance_id: entranceId,
        depth,
        prop_id: propId,
      },
    })
  }

  /** Toggle a dungeon door (entrance at depth 0, or an interior room door at
   *  depth ≥1). The server flips and broadcasts the authoritative state. */
  sendToggleDungeonDoor(entranceId: string, depth: number, doorId: number) {
    this.sendMessage({
      ToggleDungeonDoor: {
        entrance_id: entranceId,
        depth,
        door_id: doorId,
      },
    })
  }

  /** Ask the server for the current open/closed state of all of a dungeon's
   *  doors (sent on registering the dungeon, so others' open doors render). */
  sendRequestDungeonDoors(entranceId: string) {
    this.sendMessage({ RequestDungeonDoors: { entrance_id: entranceId } })
  }

  sendTorchToggle(enabled: boolean) {
    this.sendMessage({ TorchToggle: { enabled } })
  }

  sendInteractObject(objectType: string, objectId: number) {
    this.sendMessage({
      InteractObject: {
        object_type: objectType,
        object_id: objectId,
      },
    })
  }

  sendStopInteraction() {
    this.sendMessage('StopInteraction')
  }

  sendChatMessage(message: string) {
    this.sendMessage({ ChatMessage: { message } })
  }

  requestSpawnMonster(
    type: string,
    position: { x: number; y: number; z: number },
    rotation: number
  ) {
    this.sendMessage({
      RequestSpawnMonster: { monster_type: type, position, rotation },
    })
  }

  sendToggleDoor(
    houseId: string,
    roomIndex: number,
    wallDir: WallDirection,
    segmentIndex: number
  ) {
    this.sendMessage({
      ToggleDoor: {
        house_id: houseId,
        room_index: roomIndex,
        wall_dir: wallDir,
        segment_index: segmentIndex,
      },
    })
  }

  sendEquipItem(instanceId: number) {
    if (!this.isNetworkableInstanceId(instanceId, 'equip')) return
    this.sendMessage({ EquipItem: { instance_id: instanceId } })
  }

  sendUnequipItem(slot: EquipSlot) {
    this.sendMessage({ UnequipItem: { slot } })
  }

  sendDebugDropItem(itemDefId: string) {
    this.sendMessage({ DebugDropItem: { item_def_id: itemDefId } })
  }

  sendDebugSetTime(hour: number, minute: number) {
    this.sendMessage({ DebugSetTime: { hour, minute } })
  }

  sendDebugResetDungeonProps(entranceId: string) {
    this.sendMessage({ DebugResetDungeonProps: { entrance_id: entranceId } })
  }

  // Item instance ids are assigned by the server, so invalid ids must never
  // be sent back over inventory-related messages.
  private isNetworkableInstanceId(
    instanceId: number,
    operation: string
  ): boolean {
    if (!Number.isSafeInteger(instanceId) || instanceId < 0) {
      console.warn(
        `Ignoring invalid ${operation} item instance id:`,
        instanceId
      )
      return false
    }
    return true
  }

  sendDropItem(instanceId: number) {
    if (!this.isNetworkableInstanceId(instanceId, 'drop')) return
    this.sendMessage({ DropItem: { instance_id: instanceId } })
  }

  /// Sent at the pickup clip's first frame so nearby players see the crouch
  /// from the top; `sendPickupItem` follows at the grab moment.
  sendPickupStarted() {
    this.sendMessage('PickupStarted')
  }

  sendPickupItem(instanceId: number) {
    if (!this.isNetworkableInstanceId(instanceId, 'pickup')) return
    this.sendMessage({ PickupItem: { instance_id: instanceId } })
  }

  sendUseItem(instanceId: number) {
    if (!this.isNetworkableInstanceId(instanceId, 'use')) return
    this.sendMessage({ UseItem: { instance_id: instanceId } })
  }

  sendOpenShop(merchantPlayerId: number) {
    markShopRequested(merchantPlayerId)
    this.sendMessage({ OpenShop: { merchant_player_id: merchantPlayerId } })
  }

  /** Tell the server the trade window for this merchant closed, so the NPC is
   *  released from its in-place hold (see ServerMessage::TradeBusy). */
  sendCloseShop(merchantPlayerId: number) {
    this.sendMessage({ CloseShop: { merchant_player_id: merchantPlayerId } })
  }

  sendBuyItem(merchantPlayerId: number, itemDefId: string) {
    this.sendMessage({
      BuyItem: { merchant_player_id: merchantPlayerId, item_def_id: itemDefId },
    })
  }

  sendSellItem(merchantPlayerId: number, instanceId: number) {
    if (!this.isNetworkableInstanceId(instanceId, 'sell')) return
    this.sendMessage({
      SellItem: {
        merchant_player_id: merchantPlayerId,
        instance_id: instanceId,
      },
    })
  }

  sendBuybackItem(merchantPlayerId: number, entryId: number) {
    this.sendMessage({
      BuybackItem: { merchant_player_id: merchantPlayerId, entry_id: entryId },
    })
  }

  // --- Auth & character request methods ---

  /// The server refuses every message that arrives before ClientInfo, so this
  /// runs from the send path itself rather than from `onopen`: the open event
  /// and the first send resolve as separate promise continuations, and losing
  /// that race got the connection dropped.
  private ensureHandshake() {
    if (this.handshakeSent) return
    this.handshakeSent = true
    const bytes = serializeClientMessage({
      ClientInfo: {
        protocol_version: protocol_version(),
        client_kind: 'web',
        client_version: __APP_VERSION__,
      },
    })
    this.socket!.send(bytes)
  }

  private authenticateWithGoogle(googleIdToken: string): boolean {
    setApiAuthToken(googleIdToken)

    return this.sendAndSerialize({
      Authenticate: { google_id_token: googleIdToken },
    })
  }

  /// Drop cached credentials so a later reconnect can't re-auth as this user.
  /// Call on logout/kick, not on transient disconnects (which must reconnect).
  clearSession() {
    this.lastCharacterId = null
    setApiAuthToken(null)
  }

  async requestAuthentication(
    serverUrl: string,
    googleIdToken: string
  ): Promise<{
    ok: boolean
    message?: string
    accountName?: string
    characters?: AccountCharacter[]
  }> {
    await this.ensureWasm()
    this.connect(serverUrl)
    const opened = await this.waitForSocketOpen(5000)
    if (!opened) {
      return { ok: false, message: 'Failed to connect to server' }
    }

    return this.requestWithTimeout(
      8000,
      'Authentication timed out',
      (settle, onCleanup) => {
        onCleanup(
          this.authSuccess.on((payload) => {
            settle({
              ok: true,
              accountName: payload.accountName,
              characters: payload.characters,
            })
          })
        )
        onCleanup(
          this.authError.on((message) => {
            settle({ ok: false, message })
          })
        )
        return {
          send: () => this.authenticateWithGoogle(googleIdToken),
          notSentResult: { ok: false, message: 'Socket is not connected' },
        }
      }
    )
  }

  async requestCreateCharacter(
    characterName: string,
    characterClass: CharacterClass,
    gender: Gender
  ): Promise<{ ok: boolean; message?: string; character?: AccountCharacter }> {
    await this.ensureWasm()
    if (!this.isConnected()) {
      return { ok: false, message: 'Socket is not connected' }
    }

    return this.requestWithTimeout(
      8000,
      'Character creation timed out',
      (settle, onCleanup) => {
        onCleanup(
          this.characterCreated.on((character) => {
            settle({ ok: true, character })
          })
        )
        onCleanup(
          this.characterError.on((message) => {
            settle({ ok: false, message })
          })
        )
        onCleanup(
          this.authError.on((message) => {
            settle({ ok: false, message })
          })
        )
        return {
          send: () =>
            this.sendAndSerialize({
              CreateCharacter: {
                character_name: characterName,
                character_class: characterClass,
                gender,
              },
            }),
          notSentResult: { ok: false, message: 'Socket is not connected' },
        }
      }
    )
  }

  async requestDeleteCharacter(
    characterId: number
  ): Promise<{ ok: boolean; message?: string }> {
    await this.ensureWasm()
    if (!this.isConnected()) {
      return { ok: false, message: 'Socket is not connected' }
    }

    return this.requestWithTimeout(
      8000,
      'Character deletion timed out',
      (settle, onCleanup) => {
        onCleanup(
          this.characterDeleted.on(() => {
            settle({ ok: true })
          })
        )
        onCleanup(
          this.characterError.on((message) => {
            settle({ ok: false, message })
          })
        )
        onCleanup(
          this.authError.on((message) => {
            settle({ ok: false, message })
          })
        )
        return {
          send: () =>
            this.sendAndSerialize({
              DeleteCharacter: { character_id: characterId },
            }),
          notSentResult: { ok: false, message: 'Socket is not connected' },
        }
      }
    )
  }

  async requestRollCharacterStats(
    characterClass: CharacterClass,
    gender: Gender
  ): Promise<RollCharacterStatsResult> {
    await this.ensureWasm()
    if (!this.isConnected()) {
      return { ok: false, message: 'Socket is not connected' }
    }

    return this.requestWithTimeout(
      8000,
      'Stat roll timed out',
      (settle, onCleanup) => {
        onCleanup(
          this.characterStatsRolled.on((result) => {
            settle({
              ok: true,
              attributes: result.attributes,
              maxHp: result.maxHp,
            })
          })
        )
        onCleanup(
          this.characterError.on((message) => {
            settle({ ok: false, message })
          })
        )
        onCleanup(
          this.authError.on((message) => {
            settle({ ok: false, message })
          })
        )
        return {
          send: () =>
            this.sendAndSerialize({
              RollCharacterStats: { character_class: characterClass, gender },
            }),
          notSentResult: { ok: false, message: 'Socket is not connected' },
        }
      }
    )
  }

  async requestEnterGame(
    characterId: number
  ): Promise<{ ok: boolean; message?: string }> {
    await this.ensureWasm()
    if (!this.isConnected()) {
      return { ok: false, message: 'Socket is not connected' }
    }

    this.lastCharacterId = characterId
    return this.requestWithTimeout(
      8000,
      'Game entry timed out',
      (settle, onCleanup) => {
        onCleanup(
          this.joinSuccess.on(() => {
            settle({ ok: true })
          })
        )
        onCleanup(
          this.characterError.on((message) => {
            settle({ ok: false, message })
          })
        )
        onCleanup(
          this.authError.on((message) => {
            settle({ ok: false, message })
          })
        )
        return {
          send: () =>
            this.sendAndSerialize({
              EnterGame: { character_id: characterId },
            }),
          notSentResult: { ok: false, message: 'Socket is not connected' },
        }
      }
    )
  }

  // --- Connection management ---

  disconnect() {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    clearServerGameTime()
    if (this.socket) {
      this.socket.onopen = null
      this.socket.onclose = null
      this.socket.onerror = null
      this.socket.onmessage = null
      this.socket.close()
      this.socket = null
    }
  }

  private resetAllState() {
    this.disconnect()
    // Both callers are user-initiated, so the backoff earned by the previous
    // socket shouldn't be held against the new one.
    this.reconnectAttempts = 0
    resetGameStore()
    monsterManager.reset()
    remotePlayerManager.reset()
  }

  async reconnect() {
    console.log('Manual reconnection requested. Resetting state...')
    this.resetAllState()

    const serverUrl = this.lastServerUrl
    const googleIdToken = getApiAuthToken()
    const characterId = this.lastCharacterId
    if (!serverUrl || !googleIdToken || !characterId) {
      console.warn('Reconnect skipped: missing account or character context')
      return
    }

    await this.ensureWasm()
    this.connect(serverUrl)
    const opened = await this.waitForSocketOpen(5000)
    if (!opened) {
      console.warn('Reconnect failed: socket open timeout')
      return
    }

    this.requestWithTimeout<{ ok: boolean; message?: string }>(
      8000,
      'Reconnect auth timed out',
      (settle, onCleanup) => {
        onCleanup(
          this.authSuccess.on(() => {
            settle({ ok: true })
            this.sendAndSerialize({
              EnterGame: { character_id: characterId },
            })
          })
        )
        onCleanup(
          this.authError.on((message) => {
            settle({ ok: false, message })
            console.warn('Reconnect auth failed:', message)
          })
        )
        return {
          send: () => this.authenticateWithGoogle(googleIdToken),
          notSentResult: { ok: false, message: 'Socket is not connected' },
        }
      }
    )
  }

  async requestReauthenticate(): Promise<{
    ok: boolean
    message?: string
    accountName?: string
    characters?: AccountCharacter[]
  }> {
    this.resetAllState()

    const serverUrl = this.lastServerUrl
    const googleIdToken = getApiAuthToken()
    if (!serverUrl || !googleIdToken) {
      return { ok: false, message: 'Missing account context' }
    }

    await this.ensureWasm()
    this.connect(serverUrl)
    const opened = await this.waitForSocketOpen(5000)
    if (!opened) {
      return { ok: false, message: 'Failed to connect to server' }
    }

    return this.requestWithTimeout(
      8000,
      'Re-authentication timed out',
      (settle, onCleanup) => {
        onCleanup(
          this.authSuccess.on((payload) => {
            settle({
              ok: true,
              accountName: payload.accountName,
              characters: payload.characters,
            })
          })
        )
        onCleanup(
          this.authError.on((message) => {
            settle({ ok: false, message })
          })
        )
        return {
          send: () => this.authenticateWithGoogle(googleIdToken),
          notSentResult: { ok: false, message: 'Socket is not connected' },
        }
      }
    )
  }
}

export const networkManager = hmrSingleton(
  'networkManager',
  () => new NetworkManager()
)

// Notify the server whenever a trade window closes (or switches merchants), so
// the NPC it was trading with is released from its in-place hold. The window
// is opened via an explicit OpenShop; this mirrors it with a CloseShop.
let lastOpenMerchantId: number | null = null
shopSession.subscribe((session) => {
  const merchantId = session?.merchantPlayerId ?? null
  if (merchantId === lastOpenMerchantId) return
  if (lastOpenMerchantId !== null) {
    networkManager.sendCloseShop(lastOpenMerchantId)
  }
  lastOpenMerchantId = merchantId
})

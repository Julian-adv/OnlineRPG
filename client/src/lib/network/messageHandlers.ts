import { get } from 'svelte/store'
import {
  gameStore,
  updatePlayer,
  addChatMessage,
  addCombatMessage,
  addChatBubble,
  resetGameStore,
  isAdminUser,
  serverNotice,
} from '../stores/gameStore'
import type { GameState, LocalPlayer, RemotePlayer } from '../stores/gameStore'
import { Vector3 } from 'three'
import { remotePlayerManager } from '../managers/remotePlayerManager'
import { monsterManager } from '../managers/monsterManager'
import { housingManager } from '../managers/housingManager'
import { bridgeManager } from '../managers/bridgeManager'
import { objectManager } from '../managers/objectManager'
import { groundItemManager } from '../managers/groundItemManager'
import { dungeonManager } from '../managers/dungeonManager'
import { deathDropDelayQueue } from '../managers/deathDropDelay'
import { setInventory, playerGold, playerGuard } from '../stores/inventoryStore'
import { catchMessage } from './fishingMessages'
import {
  setSkills,
  applySkillXp,
  SKILL_DISPLAY_NAMES,
} from '../stores/skillsStore'
import {
  myFishingPhase,
  myStruggle,
  setStruggleRound,
  applyStruggleTension,
  upsertBobber,
  markBobberBite,
  removeBobber,
} from '../stores/fishingStore'
import { getItemDef } from '../data/itemDefs'
import {
  shopSession,
  applyDealUpdate,
  setMerchantDeals,
  wasShopRequested,
  pendingTradeOffer,
  type BuybackEntry,
} from '../stores/tradeStore'
import { editorTreeDataManager } from '../stores/editorStore'
import type { MonsterData } from '../types/Monster'
import { requestCameraReset } from '../stores/cameraStore'
import { setServerGameTime } from '../stores/timeStore'
import { whisperChatEntry } from '../chat-format'
import type { NetworkEvent } from './networkEvents'
import type {
  AccountCharacter,
  AuthSuccessPayload,
  CharacterAttributes,
  CharacterRollResult,
  ServerGroundItem,
  PositionCorrection,
  ServerMonster,
  ServerPlayer,
} from './networkTypes'

function mapBuyback(
  entries:
    | {
        entry_id: number
        item_def_id: string
        enchant: number
        price: number
      }[]
    | undefined
): BuybackEntry[] {
  return (entries ?? []).map((e) => ({
    entryId: e.entry_id,
    itemDefId: e.item_def_id,
    enchant: e.enchant,
    price: Number(e.price),
  }))
}

function toLocalPlayer(sp: ServerPlayer): LocalPlayer {
  return {
    ...sp,
    position: new Vector3(sp.position.x, sp.position.y, sp.position.z),
    rotation: sp.rotation ?? 0,
    maxHealth: sp.max_health,
    characterClass: sp.class,
    gender: sp.gender,
  }
}

function toRemotePlayer(sp: ServerPlayer): RemotePlayer {
  return {
    id: sp.id,
    name: sp.name,
    level: sp.level,
    health: sp.health,
    maxHealth: sp.max_health,
    characterClass: sp.class,
    gender: sp.gender,
    torchOn: sp.torch_on,
    floorLevel: sp.floor_level ?? 0,
    isOfficialNpc: sp.is_official_npc ?? false,
  }
}

function emitCurrentPlayerDamageInfo(
  playerId: number,
  damage: number,
  hit: boolean,
  currentHealth: number,
  delayMs: number
) {
  const emit = () => {
    const state = get(gameStore)
    if (state.currentPlayer?.id !== playerId) return

    updatePlayer(playerId, {
      lastDamageInfo: {
        damage,
        hit,
        currentHealth,
        trigger: (state.currentPlayer.lastDamageInfo?.trigger ?? 0) + 1,
      },
    })
  }

  if (delayMs > 0) {
    globalThis.setTimeout(emit, delayMs)
  } else {
    emit()
  }
}

/** Resolve object interaction for a remote player: find nearest placement, snap position/rotation. */
async function applyObjectInteraction(
  playerId: number,
  objectType: string,
  wx: number,
  wz: number
) {
  // Pickup is an animation, not a placed object: it happens wherever the
  // player is standing, so the placement search can only ever find nothing.
  // Skipping it drops two awaits and a scan of every cached region before
  // the crouch starts.
  if (objectType === 'pickup') {
    remotePlayerManager.handleInteraction(playerId, objectType, 0)
    return
  }

  await objectManager.fetchCatalog()
  const def = objectManager.getCatalogEntry(objectType)
  const anim = def?.interaction ?? objectType
  const offsetY = def?.interactOffset?.y ?? 0
  const placement = await objectManager.findNearestPlacementAsync(
    objectType,
    wx,
    wz
  )
  const pos = placement
    ? { x: placement.x, y: placement.y, z: placement.z }
    : undefined
  const rot = placement ? placement.rotation : undefined
  remotePlayerManager.handleInteraction(playerId, anim, offsetY, pos, rot)
}

/** Spawn a remote player's visual, apply any object interaction, and store it in game state. */
function addRemotePlayerToState(state: GameState, sp: ServerPlayer) {
  remotePlayerManager.initPlayer(sp.id, sp.position, sp.rotation)
  if (sp.object_type) {
    applyObjectInteraction(sp.id, sp.object_type, sp.position.x, sp.position.z)
  }
  state.otherPlayers.set(sp.id, toRemotePlayer(sp))
}

/** Remove a remote player's visual and store entry. */
function removeRemotePlayerFromState(state: GameState, playerId: number) {
  remotePlayerManager.removePlayer(playerId)
  state.otherPlayers.delete(playerId)
}

export type MessageEvents = {
  authSuccess: NetworkEvent<(payload: AuthSuccessPayload) => void>
  authError: NetworkEvent<(message: string) => void>
  joinSuccess: NetworkEvent<() => void>
  characterCreated: NetworkEvent<(character: AccountCharacter) => void>
  characterStatsRolled: NetworkEvent<(result: CharacterRollResult) => void>
  characterDeleted: NetworkEvent<(characterId: number) => void>
  characterError: NetworkEvent<(message: string) => void>
  kicked: NetworkEvent<(reason: string) => void>
  playerRespawned: NetworkEvent<(playerId: number) => void>
  interactionRejected: NetworkEvent<(reason: string) => void>
  positionCorrected: NetworkEvent<(c: PositionCorrection) => void>
}

export function handleServerMessage(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  raw: any,
  events: MessageEvents,
  disconnect: () => void
) {
  if (typeof raw === 'string') {
    return
  }

  const type = Object.keys(raw)[0]
  const data = raw[type]

  switch (type) {
    case 'AuthSuccess': {
      const characters = (data.characters as AccountCharacter[]) ?? []
      events.authSuccess.emit({
        accountName: data.account_name,
        characters,
      })
      break
    }

    case 'AuthError': {
      console.warn('Authentication error:', data.message)
      events.authError.emit(data.message)
      break
    }

    case 'JoinSuccess': {
      const serverPlayer: ServerPlayer = data.player
      console.log('Join successful, received player data:', serverPlayer)
      isAdminUser.set(data.is_admin === true)
      const player = toLocalPlayer(serverPlayer)
      gameStore.update((state) => ({
        ...state,
        currentPlayer: player,
      }))
      // Players who logged out inside a dungeon reconnect there.
      dungeonManager.syncFromFloorLevel(
        serverPlayer.floor_level ?? 0,
        serverPlayer.position.x,
        serverPlayer.position.z
      )
      events.joinSuccess.emit()
      break
    }

    case 'CharacterCreated': {
      const character: AccountCharacter = data.character
      events.characterCreated.emit(character)
      break
    }

    case 'CharacterStatsRolled': {
      const attributes: CharacterAttributes = data.attributes
      events.characterStatsRolled.emit({
        attributes,
        maxHp: data.max_hp,
      })
      break
    }

    case 'CharacterDeleted': {
      events.characterDeleted.emit(data.character_id)
      break
    }

    case 'CharacterError': {
      events.characterError.emit(data.message)
      break
    }

    case 'PlayerJoined': {
      const serverPlayer: ServerPlayer = data.player
      const player = toLocalPlayer(serverPlayer)
      let joinedName: string | null = null
      gameStore.update((state) => {
        if (!state.currentPlayer) {
          console.log('Setting current player from PlayerJoined:', player)
          return { ...state, currentPlayer: player }
        } else if (serverPlayer.id !== state.currentPlayer.id) {
          addRemotePlayerToState(state, serverPlayer)
          joinedName = serverPlayer.name
        }
        return state
      })
      if (joinedName) {
        addChatMessage({
          text: `${joinedName} joined the game`,
          sender: 'system',
        })
      }
      break
    }

    case 'PlayerAppeared': {
      const serverPlayer: ServerPlayer = data.player
      gameStore.update((state) => {
        if (serverPlayer.id !== state.currentPlayer?.id) {
          addRemotePlayerToState(state, serverPlayer)
        }
        return state
      })
      break
    }

    case 'PlayerLeft': {
      let leftName: string | null = null
      gameStore.update((state) => {
        const player = state.otherPlayers.get(data.player_id)
        removeRemotePlayerFromState(state, data.player_id)
        if (player) {
          leftName = player.name
        }
        return state
      })
      if (leftName) {
        addChatMessage({ text: `${leftName} left the game`, sender: 'system' })
      }
      break
    }

    case 'PlayerDisappeared': {
      gameStore.update((state) => {
        removeRemotePlayerFromState(state, data.player_id)
        return state
      })
      break
    }

    case 'PlayerMoved': {
      const state = get(gameStore)
      if (state.currentPlayer?.id === data.player_id) {
        break
      }
      const deckY = bridgeManager.findDeckYAt(
        data.position.x,
        data.position.z,
        null
      )
      remotePlayerManager.setTargetPosition(
        data.player_id,
        {
          x: data.position.x,
          y: deckY ?? data.position.y,
          z: data.position.z,
        },
        data.rotation
      )
      const existing = state.otherPlayers.get(data.player_id)
      if (existing && existing.floorLevel !== data.floor_level) {
        updatePlayer(data.player_id, { floorLevel: data.floor_level })
      }
      break
    }

    case 'PositionCorrected': {
      // No id to match: it only ever goes to the player it corrects.
      events.positionCorrected.emit({
        x: data.position.x,
        y: data.position.y,
        z: data.position.z,
        rotation: data.rotation,
        floorLevel: data.floor_level ?? 0,
      })
      break
    }

    case 'PlayerTeleported': {
      const state = get(gameStore)
      if (state.currentPlayer && state.currentPlayer.id === data.player_id) {
        state.currentPlayer.position.set(
          data.position.x,
          data.position.y,
          data.position.z
        )
        dungeonManager.syncFromFloorLevel(
          data.floor_level ?? 0,
          data.position.x,
          data.position.z
        )
        requestCameraReset()
        break
      }
      const tpDeckY = bridgeManager.findDeckYAt(
        data.position.x,
        data.position.z,
        null
      )
      remotePlayerManager.teleportPlayer(
        data.player_id,
        tpDeckY !== null ? { ...data.position, y: tpDeckY } : data.position,
        data.rotation
      )
      break
    }

    case 'ChatMessage': {
      const state = get(gameStore)
      const isLocal = state.currentPlayer?.id === data.player_id
      const playerName = isLocal
        ? state.currentPlayer?.name
        : (state.otherPlayers.get(data.player_id)?.name ?? 'Unknown')
      addChatMessage({
        text: data.message,
        sender: isLocal ? 'local' : 'remote',
        name: playerName,
      })
      addChatBubble(data.player_id, data.message)
      break
    }

    case 'WhisperMessage': {
      // No chat bubble — a whisper is private.
      const own = get(gameStore).currentPlayer?.name
      addChatMessage(whisperChatEntry(data.from, data.to, data.message, own))
      break
    }

    case 'SystemMessage':
      addChatMessage({ text: data.message, sender: 'system' })
      break

    case 'GameState':
      gameStore.update((state) => {
        state.otherPlayers.clear()
        remotePlayerManager.reset()
        // A list, not a map: player ids are numeric and the wasm serializer
        // rejects non-string map keys (see ServerMessage::GameState).
        const serverPlayers = data.players as ServerPlayer[]
        serverPlayers.forEach((serverPlayer) => {
          if (serverPlayer.id !== state.currentPlayer?.id) {
            const player = toRemotePlayer(serverPlayer)
            remotePlayerManager.initPlayer(
              serverPlayer.id,
              serverPlayer.position,
              serverPlayer.rotation
            )
            if (serverPlayer.object_type) {
              applyObjectInteraction(
                serverPlayer.id,
                serverPlayer.object_type,
                serverPlayer.position.x,
                serverPlayer.position.z
              )
            }
            state.otherPlayers.set(serverPlayer.id, player)
          }
        })
        return state
      })

      monsterManager.reset()
      if (data.monsters) {
        Object.values(data.monsters as Record<string, ServerMonster>).forEach(
          (monster) => {
            monsterManager.spawnWithId(
              monster.id,
              monster.monster_type as MonsterData['type'],
              monster.position,
              monster.owner_id,
              monster.health,
              monster.max_health,
              monster.floor_level,
              monster.aggressive
            )
          }
        )
      }

      groundItemManager.reset()
      if (data.ground_items) {
        ;(data.ground_items as ServerGroundItem[]).forEach((item) => {
          groundItemManager.spawn(item)
        })
      }
      break

    case 'GameTimeSync': {
      setServerGameTime({
        year: data.datetime.year,
        month: data.datetime.month,
        day: data.datetime.day,
        hour: data.datetime.hour,
        minute: data.datetime.minute,
        isNight: data.is_night,
      })
      break
    }

    case 'MonsterSpawned': {
      const monster: ServerMonster = data.monster
      monsterManager.spawnWithId(
        monster.id,
        monster.monster_type as MonsterData['type'],
        monster.position,
        monster.owner_id,
        monster.health,
        monster.max_health,
        monster.floor_level,
        monster.aggressive
      )
      break
    }

    case 'SpawnMonsterRequest': {
      // Server asks us to spawn a monster near the local player; pick a valid
      // grassland spot away from water/towns and request it.
      monsterManager.tryAmbientSpawn(data.monster_type)
      break
    }

    case 'NoSpawnZones':
      monsterManager.setNoSpawnZones(data.zones ?? [])
      break

    case 'MonsterAssigned': {
      const assigned: ServerMonster = data.monster
      // May be a reassignment of a monster we already track (dungeon
      // owner handover): update the owner and (re)create our brain.
      monsterManager.adoptOwnership(
        assigned.id,
        assigned.monster_type as MonsterData['type'],
        assigned.position,
        assigned.owner_id,
        assigned.health,
        assigned.max_health,
        assigned.floor_level,
        assigned.aggressive
      )
      break
    }

    case 'MonsterMoved':
      monsterManager.updateMonsterFromNetwork(
        data.monster_id,
        data.position,
        data.rotation,
        data.state,
        data.target_position
      )
      break

    case 'MonsterRemoved':
      monsterManager.remove(data.monster_id)
      break

    case 'MonsterDead':
      monsterManager.handleMonsterDead(
        data.monster_id,
        data.dropped_weapon_item_def_id
      )
      break

    case 'PlayerAttacked': {
      remotePlayerManager.handleAttack(data.player_id)

      const gameState = get(gameStore)
      const isLocalAttacker = gameState.currentPlayer?.id === data.player_id
      const attackerName = isLocalAttacker
        ? 'You'
        : gameState.otherPlayers.get(data.player_id)?.name || 'Unknown'

      addCombatMessage({
        text: data.hit
          ? `rolled ${data.roll}: HIT for ${data.damage} damage!`
          : `rolled ${data.roll}: MISSED!`,
        sender: isLocalAttacker ? 'local' : 'remote',
        name: attackerName,
        hit: data.hit,
      })

      monsterManager.handleMonsterAttacked(
        data.monster_id,
        data.player_id,
        data.hit,
        data.damage
      )
      break
    }

    case 'MonsterProvoked':
      monsterManager.handleMonsterProvoked(data.monster_id, data.player_id)
      break

    case 'MonsterAttackedPlayer': {
      const gameState = get(gameStore)
      const isCurrentPlayer = gameState.currentPlayer?.id === data.player_id
      const monster = monsterManager.monsters.get(data.monster_id)
      if (monster?.ownerId !== gameState.currentPlayer?.id) {
        monsterManager.handleMonsterAttackStarted(data.monster_id, 250)
      }

      if (isCurrentPlayer) {
        emitCurrentPlayerDamageInfo(
          data.player_id,
          data.damage,
          data.hit,
          data.current_health,
          monsterManager.getMonsterAttackDamageTextDelayMs(data.monster_id)
        )
      }

      updatePlayer(data.player_id, {
        health: data.current_health,
      })

      const monsterTargetName = isCurrentPlayer
        ? 'You'
        : (gameState.otherPlayers.get(data.player_id)?.name ?? 'Unknown')
      addCombatMessage({
        text: data.hit
          ? `rolled ${data.roll}: HIT ${monsterTargetName} for ${data.damage} damage!`
          : `rolled ${data.roll}: MISSED!`,
        sender: 'system',
        name: 'Monster',
        hit: data.hit,
      })
      break
    }

    case 'PlayerDead': {
      console.log('Player dead:', data.player_id)
      const gameState = get(gameStore)
      const isDeadCurrentPlayer = gameState.currentPlayer?.id === data.player_id
      const deadPlayerName = isDeadCurrentPlayer
        ? 'You'
        : (gameState.otherPlayers.get(data.player_id)?.name ?? 'Unknown')
      addCombatMessage({
        text: `${deadPlayerName === 'You' ? 'You have' : deadPlayerName + ' has'} been slain!`,
        sender: 'system',
      })

      if (!isDeadCurrentPlayer) {
        remotePlayerManager.handleDead(data.player_id)
      }
      break
    }

    case 'Kicked': {
      console.warn('Kicked from server:', data.reason)
      events.kicked.emit(data.reason)
      resetGameStore()
      monsterManager.reset()
      remotePlayerManager.reset()
      disconnect()
      break
    }

    case 'ServerNotice': {
      serverNotice.set(data.message ?? null)
      break
    }

    case 'PlayerRespawned': {
      const serverPlayer: ServerPlayer = data.player
      console.log('Player respawned:', serverPlayer.id)
      const gameState = get(gameStore)
      const isCurrentPlayerRespawned =
        gameState.currentPlayer?.id === serverPlayer.id

      if (isCurrentPlayerRespawned) {
        const respawnPosition = new Vector3(
          serverPlayer.position.x,
          serverPlayer.position.y,
          serverPlayer.position.z
        )
        updatePlayer(serverPlayer.id, {
          position: respawnPosition,
          health: serverPlayer.health,
          maxHealth: serverPlayer.max_health,
        })
        // Death exits the dungeon: respawn is always on the surface.
        dungeonManager.syncFromFloorLevel(
          serverPlayer.floor_level ?? 0,
          serverPlayer.position.x,
          serverPlayer.position.z
        )
        // Drop the dungeon's monsters we left behind. The server already
        // despawned/handed them off, but its MonsterRemoved is filtered to
        // the dungeon floor we just left, so we never get it — purge by
        // floor to avoid undamageable "ghost" monsters on re-entry.
        monsterManager.removeMonstersNotOnFloor(serverPlayer.floor_level ?? 0)
        requestCameraReset()
        addChatMessage({ text: 'You have been revived.', sender: 'system' })
      } else {
        updatePlayer(serverPlayer.id, {
          health: serverPlayer.health,
          maxHealth: serverPlayer.max_health,
        })
        addChatMessage({
          text: `${serverPlayer.name} has been revived.`,
          sender: 'system',
        })
        remotePlayerManager.handleRespawn(
          serverPlayer.id,
          serverPlayer.position,
          serverPlayer.rotation
        )
      }
      events.playerRespawned.emit(serverPlayer.id)
      break
    }

    case 'PlayerHealthUpdate': {
      const gameState = get(gameStore)
      const isCurrentPlayer = gameState.currentPlayer?.id === data.player_id

      let regenInfo = undefined
      if (isCurrentPlayer && gameState.currentPlayer) {
        const diff = data.health - gameState.currentPlayer.health
        if (diff > 0) {
          const prevTrigger =
            gameState.currentPlayer.lastRegenInfo?.trigger ?? 0
          regenInfo = {
            damage: diff,
            hit: true,
            trigger: prevTrigger + 1,
          }
        }
      }

      updatePlayer(data.player_id, {
        health: data.health,
        maxHealth: data.max_health,
        ...(isCurrentPlayer ? { lastRegenInfo: regenInfo } : {}),
      })
      break
    }

    case 'PlayerTorchToggled': {
      const state = get(gameStore)
      if (state.currentPlayer?.id === data.player_id) {
        break
      }
      updatePlayer(data.player_id, { torchOn: data.enabled })
      break
    }

    case 'PlayerInteractionChanged': {
      const state = get(gameStore)
      if (state.currentPlayer?.id === data.player_id) {
        break
      }
      const ft: string | null = data.object_type ?? null
      if (ft) {
        const rp = remotePlayerManager.players.get(data.player_id)
        const wx = rp?.position.x ?? 0
        const wz = rp?.position.z ?? 0
        applyObjectInteraction(data.player_id, ft, wx, wz)
      } else {
        remotePlayerManager.handleStopInteraction(data.player_id)
      }
      break
    }

    case 'InteractionRejected':
      events.interactionRejected.emit(data.reason)
      break

    case 'DungeonChestOpened': {
      const state = get(gameStore)
      const isMe = state.currentPlayer?.id === data.player_id
      const who = isMe
        ? 'You'
        : (state.otherPlayers.get(data.player_id)?.name ?? 'Someone')
      const items = (data.item_def_ids as string[]).join(', ')
      addChatMessage({
        text: `${who} opened the treasure chest: ${items} + ${data.gold} gold!`,
        sender: 'system',
      })
      break
    }

    case 'DungeonPropsState':
      dungeonManager.setPropsState(
        data.entrance_id,
        data.depth,
        data.broken,
        data.opened
      )
      break

    case 'DungeonPropBroken':
      dungeonManager.markPropBroken(data.entrance_id, data.depth, data.prop_id)
      break

    case 'DungeonPropOpened':
      dungeonManager.markPropOpened(data.entrance_id, data.depth, data.prop_id)
      break

    case 'DungeonDoorToggled':
      dungeonManager.applyDoorToggle(
        data.entrance_id,
        data.depth,
        data.door_id,
        data.is_open
      )
      break

    case 'DungeonDoorsState':
      dungeonManager.applyDoorsSnapshot(data.entrance_id, data.doors)
      break

    case 'HouseSpawned':
      housingManager.handleRemoteHouseSpawned(data.house)
      break

    case 'HouseUpdated':
      housingManager.handleRemoteHouseSpawned(data.house)
      break

    case 'TreeTilesInvalidated': {
      const treeDataManager = get(editorTreeDataManager)
      if (treeDataManager) void treeDataManager.refreshTiles(data.tiles ?? [])
      break
    }

    case 'HouseRemoved':
      housingManager.handleRemoteHouseRemoved(data.house_id)
      break

    case 'HousesInArea':
      housingManager.handleRemoteHousesBatch(data.houses)
      break

    case 'DoorToggled':
      housingManager.handleDoorToggled(
        data.house_id,
        data.room_index,
        data.wall_dir,
        data.segment_index,
        data.is_open
      )
      break

    case 'InventoryState':
    case 'InventoryUpdated':
      setInventory(data.inventory)
      break

    case 'GroundItemSpawned': {
      const item = data.item as ServerGroundItem
      deathDropDelayQueue.handleSpawn(
        data.source_monster_id as string | undefined,
        item.instance_id,
        () =>
          groundItemManager.spawn(item, {
            animateSpawn: true,
          })
      )
      break
    }

    case 'GroundItemAppeared':
      groundItemManager.spawn(data.item as ServerGroundItem)
      break

    case 'GroundItemRemoved':
      deathDropDelayQueue.cancelSpawn(data.instance_id)
      groundItemManager.remove(data.instance_id)
      break

    case 'ShopState': {
      const session = {
        merchantPlayerId: data.merchant_player_id,
        merchantName: data.merchant_name,
        catalog: data.catalog ?? [],
        sellRatePercent: data.sell_rate_percent,
        wishlist: data.wishlist ?? [],
        stock: (data.stock ?? []).map(
          (entry: { item_def_id: string; quantity: number }) => ({
            itemDefId: entry.item_def_id,
            quantity: entry.quantity,
          })
        ),
        buyback: mapBuyback(data.buyback),
      }
      setMerchantDeals(data.merchant_player_id, data.active_deals ?? [])
      // Open directly only when the player asked for this shop (or it's a
      // refresh of the one already on screen). An NPC-pushed open_trade is
      // an *offer*: the window covers much of the screen, so it just shows
      // a small accept/decline toast instead of hijacking the view.
      const current = get(shopSession)
      if (
        wasShopRequested(data.merchant_player_id) ||
        current?.merchantPlayerId === data.merchant_player_id
      ) {
        shopSession.set(session)
      } else {
        pendingTradeOffer.set({ session, offeredAt: Date.now() })
      }
      break
    }

    case 'GoldUpdate':
      playerGold.set(Number(data.gold))
      break

    case 'GuardUpdated':
      playerGuard.set(Number(data.guard))
      break

    case 'GoldGained': {
      const state = get(gameStore)
      const playerId = state.currentPlayer?.id
      if (playerId) {
        updatePlayer(playerId, {
          lastGoldInfo: {
            amount: Number(data.amount),
            trigger: (state.currentPlayer?.lastGoldInfo?.trigger ?? 0) + 1,
          },
        })
      }
      break
    }

    case 'TradeError':
      addChatMessage({ text: data.message, sender: 'system' })
      break

    case 'DealUpdated':
      applyDealUpdate(
        data.merchant_player_id,
        data.item_def_id,
        data.kind,
        data.modifier_pct,
        data.expires_in_secs
      )
      break

    case 'BuybackUpdated':
      shopSession.update((session) =>
        session && session.merchantPlayerId === data.merchant_player_id
          ? { ...session, buyback: mapBuyback(data.buyback) }
          : session
      )
      break

    case 'XpGained': {
      const gameState = get(gameStore)
      const previousPlayer = gameState.currentPlayer
      const previousLevel =
        previousPlayer && previousPlayer.id === data.player_id
          ? previousPlayer.level
          : null
      const isCurrentPlayer = previousPlayer?.id === data.player_id
      const newTotalXp = Number(data.total_xp)
      const xpLost = Number(data.xp_lost ?? 0)

      let regenInfo = undefined
      if (isCurrentPlayer && previousPlayer) {
        const diff = data.current_hp - previousPlayer.health
        if (diff > 0) {
          const prevTrigger = previousPlayer.lastRegenInfo?.trigger ?? 0
          regenInfo = {
            damage: diff,
            hit: true,
            trigger: prevTrigger + 1,
          }
        }
      }

      updatePlayer(data.player_id, {
        level: data.new_level,
        totalXp: newTotalXp,
        health: data.current_hp,
        maxHealth: data.max_hp,
        ...(isCurrentPlayer ? { lastRegenInfo: regenInfo } : {}),
      })
      if (data.xp_amount > 0) {
        addCombatMessage({
          text: `You gained ${data.xp_amount} XP.`,
          sender: 'local',
        })
      } else if (previousLevel !== null) {
        if (xpLost > 0) {
          addCombatMessage({
            text: `Death penalty: You lost ${xpLost} XP.`,
            sender: 'local',
          })
        } else {
          addCombatMessage({ text: 'Death penalty applied.', sender: 'local' })
        }
      }
      if (data.leveled_up) {
        addCombatMessage({
          text: `Level up! You are now level ${data.new_level}.`,
          sender: 'local',
        })
      } else if (previousLevel !== null && data.new_level < previousLevel) {
        addCombatMessage({
          text: `Level down. You are now level ${data.new_level}.`,
          sender: 'local',
        })
      }
      break
    }

    case 'SkillsUpdate':
      setSkills(data.skills)
      break

    case 'FishingCasted': {
      upsertBobber(data.player_id, data.position)
      if (get(gameStore).currentPlayer?.id === data.player_id) {
        myFishingPhase.set('casting')
        addCombatMessage({ text: 'You cast your line.', sender: 'local' })
      }
      break
    }

    case 'FishingBite': {
      markBobberBite(data.player_id)
      if (get(gameStore).currentPlayer?.id === data.player_id) {
        myFishingPhase.set('bite')
        addCombatMessage({
          text: 'Something bites! Hook it!',
          sender: 'local',
        })
      }
      break
    }

    case 'FishingStruggleRound': {
      if (get(gameStore).currentPlayer?.id === data.player_id) {
        myFishingPhase.set('struggle')
        setStruggleRound({
          round: data.round,
          totalRounds: data.total_rounds,
          fishState: data.fish_state,
          respondWithinMs: data.respond_within_ms,
          tension: data.tension_pct,
          startedAt: performance.now(),
        })
      }
      break
    }

    case 'FishingRoundResult': {
      if (get(gameStore).currentPlayer?.id === data.player_id) {
        applyStruggleTension(data.tension_pct)
      }
      break
    }

    case 'FishingEnded': {
      removeBobber(data.player_id)
      const isSelf = get(gameStore).currentPlayer?.id === data.player_id
      // Bystander celebration: everyone in radius hears about a trophy.
      if (!isSelf && data.outcome?.Caught?.trophy) {
        const { item_def_id, size_cm } = data.outcome.Caught
        const who =
          get(gameStore).otherPlayers.get(data.player_id)?.name ?? 'Someone'
        const fishName = getItemDef(item_def_id)?.name ?? item_def_id
        addCombatMessage({
          text: `${who} landed a trophy ${fishName} — ${size_cm} cm!`,
          sender: 'local',
        })
      }
      if (isSelf) {
        myFishingPhase.set('idle')
        myStruggle.set(null)
        const outcome = data.outcome
        if (outcome === 'Escaped') {
          addCombatMessage({ text: 'The fish got away.', sender: 'local' })
        } else if (outcome === 'Aborted') {
          addCombatMessage({ text: 'You reel in your line.', sender: 'local' })
        } else if (outcome?.Caught) {
          const { item_def_id, size_cm, trophy } = outcome.Caught
          addCombatMessage({
            text: catchMessage(
              getItemDef(item_def_id),
              item_def_id,
              size_cm,
              trophy
            ),
            sender: 'local',
          })
        }
      }
      break
    }

    case 'FishingError':
      addCombatMessage({ text: data.message, sender: 'local' })
      break

    case 'SkillXpGained': {
      const skillId = data.skill as import('../stores/skillsStore').SkillId
      applySkillXp(skillId, Number(data.total_xp), data.new_level)
      const skillName = SKILL_DISPLAY_NAMES[skillId] ?? skillId
      addCombatMessage({
        text: `You gained ${data.xp_amount} ${skillName} XP.`,
        sender: 'local',
      })
      if (data.leveled_up) {
        addCombatMessage({
          text: `${skillName} is now level ${data.new_level}!`,
          sender: 'local',
        })
      }
      break
    }
  }
}

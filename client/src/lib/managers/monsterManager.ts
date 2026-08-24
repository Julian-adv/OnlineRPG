import { SvelteMap } from 'svelte/reactivity'
import { hmrSingleton } from '../utils/hmr'
import * as THREE from 'three'
import { networkManager } from '../network/socket'
import { get } from 'svelte/store'
import { gameStore, type GameState } from '../stores/gameStore'
import { inventoryStore } from '../stores/inventoryStore'
import { remotePlayerManager } from './remotePlayerManager'
import type { MonsterData } from '../types/Monster'
import { getMonsterDef } from '../data/monsterDefs'
import { getItemDef } from '../data/itemDefs'
import {
  getMaterialHitSoundUrl,
  getMaterialMissSoundUrl,
} from '../data/materialImpactSounds'
import { dungeonManager } from './dungeonManager'
import type { Position } from '../utils/movementUtils'
import type { TerrainHeightManager } from './terrainHeightManager'
import {
  playMonsterDeathSound,
  playSwordHitSound,
  playSwordMissSound,
} from './sfxManager'
import { clearXpArrival, releaseXpArrival } from './xpArrival'
import { shortestWrappedDeltaX, wrapWorldX } from '../terrain/world-wrap'
import {
  PLAYER_ATTACK_DAMAGE_TEXT_DELAY_MS,
  DEFAULT_MONSTER_ATTACK_IMPACT_DELAY_MS,
  DEFAULT_MONSTER_ATTACK_COOLDOWN_MS,
  PLAYER_ATTACK_IMPACT_DELAY_MS,
  SWORD_MISS_DELAY_MS,
} from '../data/combatTiming'
import {
  ai_load_behavior_trees,
  ai_create_brain,
  ai_remove_brain,
  ai_tick_brain,
  ai_handle_hit,
  ai_handle_death,
  ai_apply_authoritative_position,
} from '../wasm/onlinerpg_shared'
import behaviorTreesJson from '../../../../data-src/behavior_trees.json'
import monstersJson from '../../../../data/monsters.json'

type MonsterState = MonsterData['state']

interface AiCommand {
  type: 'Move' | 'Attack'
  monster_id: string
  position?: { x: number; y: number; z: number }
  rotation?: number
  state?: MonsterState
  target_position?: { x: number; y: number; z: number }
  target_player_id?: number
}

interface TickResult {
  commands: AiCommand[]
  position: { x: number; y: number; z: number }
  rotation: number
  state: MonsterState
}

const DEFAULT_MONSTER_BEHAVIOR = 'brave'
// Behavior tree for proactive (선공형) monsters; acquires targets on sight.
// Overrides the monster type's default when the spawn is flagged aggressive.
// Must match shared monster_ai::AGGRESSIVE_BEHAVIOR and a tree in
// data-src/behavior_trees.json.
const AGGRESSIVE_MONSTER_BEHAVIOR = 'aggressive'
const MONSTER_POSITION_EPSILON = 0.001
// Backstop for a pending death whose hit clip never reports completion.
const DEAD_PENDING_TIMEOUT_MS = 2000

class MonsterManager {
  monsters = new SvelteMap<string, MonsterData>()
  heightManager: TerrainHeightManager | null = null
  private templatesLoaded = false

  /** Ground height on the monster's floor, or null when the terrain tile
   *  isn't streamed in — reporting the brain's stale Y then would sink the
   *  monster to sea level and get the move refused. `fallbackY` covers the
   *  floors that have no sample of their own. */
  private monsterGroundYOrNull(
    monster: MonsterData,
    x: number,
    z: number,
    fallbackY: number
  ): number | null {
    const fl = monster.floorLevel ?? 0
    if (fl < 0) {
      return dungeonManager.floorHeightAt(-fl, x, z) ?? fallbackY
    }
    if (!this.heightManager) return fallbackY
    return this.heightManager.groundYOrNull(x, z)
  }

  private monsterGroundY(monster: MonsterData, x: number, z: number): number {
    const y = monster.position.y
    return this.monsterGroundYOrNull(monster, x, z, y) ?? y
  }

  /** Ground-resolved copy of `position`, or null when the tile isn't streamed
   *  in yet — the hold sentinel `processAiCommands` reports on. */
  private snapToMonsterGround(
    monster: MonsterData,
    position: { x: number; y: number; z: number }
  ): Position | null {
    const y = this.monsterGroundYOrNull(
      monster,
      position.x,
      position.z,
      position.y
    )
    if (y === null) return null
    return { x: position.x, y, z: position.z }
  }

  private ensureTemplatesLoaded() {
    if (!this.templatesLoaded) {
      ai_load_behavior_trees(JSON.stringify(behaviorTreesJson))
      this.templatesLoaded = true
    }
  }

  findMeshPosition(
    monsterId: string,
    meshes: THREE.Group[]
  ): Position | undefined {
    for (const group of meshes) {
      if (group) {
        let found = false
        group.traverse((child) => {
          if (child.userData.monsterId === monsterId) {
            found = true
          }
        })
        if (found) {
          return {
            x: group.position.x,
            y: group.position.y,
            z: group.position.z,
          }
        }
      }
    }
    return undefined
  }

  /**
   * Behavior tree a monster we own should run. Aggressive (선공형) spawns
   * acquire targets on sight, overriding the type's default timid/brave tree.
   */
  private resolveBehavior(
    type: MonsterData['type'],
    aggressive?: boolean
  ): string {
    if (aggressive) return AGGRESSIVE_MONSTER_BEHAVIOR
    const monsterDef = (monstersJson as Record<string, { behavior?: string }>)[
      type
    ]
    return monsterDef?.behavior ?? DEFAULT_MONSTER_BEHAVIOR
  }

  /**
   * MonsterAssigned handler: either a fresh spawn assigned to us or an
   * ownership handover of a monster we already track (dungeon floors
   * reassign AI when the previous owner leaves).
   */
  adoptOwnership(
    id: string,
    type: MonsterData['type'],
    position: { x: number; y: number; z: number },
    ownerId?: number,
    health?: number,
    maxHealth?: number,
    floorLevel?: number,
    aggressive?: boolean
  ) {
    const existing = this.monsters.get(id)
    if (!existing) {
      this.spawnWithId(
        id,
        type,
        position,
        ownerId,
        health,
        maxHealth,
        floorLevel,
        aggressive
      )
      return
    }
    existing.ownerId = ownerId
    if (floorLevel !== undefined) existing.floorLevel = floorLevel
    this.monsters.set(id, { ...existing })

    const myPlayerId = get(gameStore).currentPlayer?.id
    if (ownerId === myPlayerId && existing.state !== 'dead') {
      // Recreate the brain from the monster's live state.
      ai_remove_brain(id)
      const def = getMonsterDef(type)
      this.ensureTemplatesLoaded()
      const fl = existing.floorLevel ?? 0
      ai_create_brain({
        monsterId: id,
        monsterType: type,
        position: existing.position,
        health: existing.health,
        maxHealth: existing.maxHealth,
        walkSpeed: def?.walkSpeed ?? 1,
        runSpeed: def?.runSpeed ?? 8,
        attackRange: def?.attackRange ?? 2,
        chaseRange: def?.chaseRange ?? 25,
        attackCooldown:
          def?.attackCooldown ?? DEFAULT_MONSTER_ATTACK_COOLDOWN_MS,
        behavior: this.resolveBehavior(type, aggressive),
        pathFloor:
          fl < 0 && dungeonManager.active
            ? dungeonManager.passabilityFloor(-fl)
            : 0,
      })
    }
  }

  spawnWithId(
    id: string,
    type: MonsterData['type'],
    position: { x: number; y: number; z: number },
    ownerId?: number,
    health?: number,
    maxHealth?: number,
    floorLevel?: number,
    aggressive?: boolean
  ) {
    if (this.monsters.has(id)) return

    const def = getMonsterDef(type)
    // Server is authoritative for HP and always sends health/max_health on
    // spawn; the constant is only a defensive fallback.
    const hp = health ?? 10
    const maxHp = maxHealth ?? 10

    this.monsters.set(id, {
      id,
      type,
      position,
      rotation: 0,
      state: 'idle',
      ownerId,
      moveSpeed: def?.walkSpeed ?? 1,
      stateTimer: 0,
      attackCounter: 0,
      hitCounter: 0,
      health: hp,
      maxHealth: maxHp,
      spawnPosition: { ...position },
      floorLevel: floorLevel ?? 0,
    })

    // Create WASM brain for owned monsters
    const gameState = get(gameStore)
    const myPlayerId = gameState.currentPlayer?.id
    if (ownerId === myPlayerId) {
      this.ensureTemplatesLoaded()
      const behavior = this.resolveBehavior(type, aggressive)
      // Dungeon monsters path on their depth's passability floor so the
      // maze walls apply; surface monsters use the open overworld (0).
      const fl = floorLevel ?? 0
      const pathFloor =
        fl < 0 && dungeonManager.active
          ? dungeonManager.passabilityFloor(-fl)
          : 0
      ai_create_brain({
        monsterId: id,
        monsterType: type,
        position,
        health: hp,
        maxHealth: maxHp,
        walkSpeed: def?.walkSpeed ?? 1,
        runSpeed: def?.runSpeed ?? 8,
        attackRange: def?.attackRange ?? 2,
        chaseRange: def?.chaseRange ?? 25,
        attackCooldown:
          def?.attackCooldown ?? DEFAULT_MONSTER_ATTACK_COOLDOWN_MS,
        behavior,
        pathFloor,
      })
    }
  }

  remove(id: string) {
    const monster = this.monsters.get(id)
    const gameState = get(gameStore)
    if (monster?.ownerId === gameState.currentPlayer?.id) {
      ai_remove_brain(id)
    }
    this.monsters.delete(id)
  }

  // Whether a killing blow should play the hit reaction before the death clip.
  // Defaults to true; monsters with an awkward hit clip opt out via the def.
  private deathPlaysHitFor(monster: MonsterData): boolean {
    return getMonsterDef(monster.type)?.deathPlaysHit ?? true
  }

  private playPendingSwordHitSound(monster: MonsterData) {
    if (!monster.pendingSwordHitSoundUrl) return

    playSwordHitSound(monster.pendingSwordHitSoundUrl)
    monster.pendingSwordHitSoundUrl = undefined
  }

  // Rides the killing blow's contact frame; the death clip lands much later.
  private playDeathSound(monster: MonsterData) {
    const url = getMonsterDef(monster.type)?.deathSound
    if (url) playMonsterDeathSound(url)
  }

  /** This kill is still on its way down (waiting out the blade's impact or
   *  the hit reaction), so its XP has a moment to wait for. */
  isDeathPending(id: string): boolean {
    return this.monsters.get(id)?.isDeadPending === true
  }

  handleMonsterDead(id: string, droppedWeaponItemDefId?: string | null) {
    const monster = this.monsters.get(id)
    if (monster) {
      ai_handle_death(id)
      monster.droppedWeaponItemDefId = droppedWeaponItemDefId ?? undefined
      const deathPlaysHit = this.deathPlaysHitFor(monster)
      // If we are waiting for an impact, delay the visual death
      if (monster.impactDelay && monster.impactDelay > 0) {
        monster.isDeadPending = true
        monster.deadPendingTimer = 0
      } else if (
        monster.state === 'hit' &&
        monster.isLastHitSuccess &&
        deathPlaysHit
      ) {
        this.playDeathSound(monster)
        monster.isDeadPending = true
        // The hit clip may have already finished (clamped, no further
        // 'finished' event) — restart it so its completion re-arms the death.
        this.restartHitClip(monster)
      } else {
        // Otherwise die immediately
        this.playDeathSound(monster)
        this.applyMonsterPose(monster, { state: 'dead' })
        monster.stateTimer = 0
      }
      this.monsters.set(id, { ...monster })
    }
  }

  handleMonsterHitFinished(id: string) {
    const monster = this.monsters.get(id)
    if (!monster?.isDeadPending || monster.state !== 'hit') return

    this.finishPendingDeath(monster)
  }

  private finishPendingDeath(monster: MonsterData) {
    this.applyMonsterPose(monster, { state: 'dead' })
    monster.stateTimer = 0
    monster.isDeadPending = false
    this.monsters.set(monster.id, { ...monster })
  }

  // Restarts the flinch clip (a bump forces the component to replay it even
  // when the state is already 'hit') and re-opens the pending-death window.
  private restartHitClip(monster: MonsterData) {
    monster.hitCounter = (monster.hitCounter ?? 0) + 1
    monster.deadPendingTimer = 0
  }

  handleMonsterAttacked(
    monsterId: string,
    playerId: number,
    hit: boolean,
    damage: number
  ) {
    const monster = this.monsters.get(monsterId)
    if (!monster || monster.state === 'dead') return

    // Set impact delay for the shared player slash animation to land.
    monster.impactDelay = PLAYER_ATTACK_IMPACT_DELAY_MS
    monster.targetPlayerId = playerId
    monster.isLastHitSuccess = hit
    const isLocalPlayerAttack = playerId === get(gameStore).currentPlayer?.id
    const weaponItemDefId = isLocalPlayerAttack
      ? get(inventoryStore).equipped.main_hand?.item_def_id
      : undefined
    const weaponMaterial = weaponItemDefId
      ? getItemDef(weaponItemDefId)?.material
      : undefined
    if (hit && isLocalPlayerAttack) {
      const monsterMaterial = getMonsterDef(monster.type)?.material
      monster.pendingSwordHitSoundUrl = getMaterialHitSoundUrl(
        weaponMaterial,
        monsterMaterial
      )
    } else {
      monster.pendingSwordHitSoundUrl = undefined
    }
    // Every swing that misses whooshes, another player's included; their
    // weapon is unknown here so it falls back to the default miss sound.
    if (!hit) {
      playSwordMissSound(
        getMaterialMissSoundUrl(weaponMaterial),
        SWORD_MISS_DELAY_MS
      )
    }
    // Temporarily store damage to show at impact
    monster.pendingDamage = damage
    if (isLocalPlayerAttack) {
      monster.pendingDamageText = {
        delay: PLAYER_ATTACK_DAMAGE_TEXT_DELAY_MS,
        damage,
        hit,
      }
    }

    // Trigger reactivity
    this.monsters.set(monsterId, { ...monster })
  }

  handleMonsterProvoked(monsterId: string, playerId: number) {
    const monster = this.monsters.get(monsterId)
    if (!monster || monster.state === 'dead') return

    monster.targetPlayerId = playerId
    if (monster.ownerId === get(gameStore).currentPlayer?.id) {
      const commands = ai_handle_hit(monster.id, playerId, false, 0) ?? []
      this.processAiCommands(monster, commands)
    }
  }

  handleMonsterAttackStarted(monsterId: string, dedupeWindowMs = 0) {
    const monster = this.monsters.get(monsterId)
    if (!monster || monster.state === 'dead') return

    const now = globalThis.performance?.now() ?? Date.now()
    if (
      dedupeWindowMs > 0 &&
      monster.lastAttackStartedAt !== undefined &&
      now - monster.lastAttackStartedAt < dedupeWindowMs
    ) {
      return
    }

    this.applyMonsterPose(monster, { state: 'attack' })
    monster.attackCounter = (monster.attackCounter ?? 0) + 1
    monster.lastAttackStartedAt = now
    this.monsters.set(monsterId, { ...monster })
  }

  getMonsterAttackDamageTextDelayMs(monsterId: string) {
    const monster = this.monsters.get(monsterId)
    if (!monster) return DEFAULT_MONSTER_ATTACK_IMPACT_DELAY_MS

    const def = getMonsterDef(monster.type)
    return (
      def?.attackDamageTextDelay ??
      def?.attackImpactDelay ??
      DEFAULT_MONSTER_ATTACK_IMPACT_DELAY_MS
    )
  }

  // Bump the floating damage number above a monster's head. The trigger counter
  // is what DamageText watches to spawn a new text item.
  private emitDamageText(monster: MonsterData, damage: number, hit: boolean) {
    monster.lastDamageInfo = {
      damage,
      hit,
      trigger: (monster.lastDamageInfo?.trigger || 0) + 1,
    }
  }

  reset() {
    // Remove all brains
    for (const id of this.monsters.keys()) {
      ai_remove_brain(id)
    }
    this.monsters.clear()
    clearXpArrival()
  }

  update(deltaTime: number) {
    // FSM & Movement Logic
    const gameState = get(gameStore)
    const myPlayerId = gameState.currentPlayer?.id
    const nearbyPlayers = this.buildNearbyPlayers(gameState)

    for (const monster of this.monsters.values()) {
      // Keep non-owned monster Y aligned with its floor's ground (owned
      // monsters get Y from TickResult)
      if (monster.ownerId !== myPlayerId) {
        const terrainY = this.monsterGroundY(
          monster,
          monster.position.x,
          monster.position.z
        )
        if (
          Math.abs(monster.position.y - terrainY) > MONSTER_POSITION_EPSILON
        ) {
          this.applyMonsterPose(monster, {
            position: { ...monster.position, y: terrainY },
          })
        }
      }

      let impactJustExpired = false
      let damageTextFired = false

      // Impact Delay Handling (Global for all clients to keep visuals synced)
      if (monster.impactDelay !== undefined && monster.impactDelay > 0) {
        monster.impactDelay -= deltaTime
        if (monster.impactDelay <= 0) {
          monster.impactDelay = 0
          impactJustExpired = true

          if (monster.isDeadPending) {
            this.playDeathSound(monster)
            // Fatal impact: optionally play hit first, then transition to death
            // when the hit clip reports completion. Monsters with an awkward hit
            // clip (deathPlaysHit=false) go straight to the death clip.
            const leadWithHit =
              monster.isLastHitSuccess && this.deathPlaysHitFor(monster)
            this.applyMonsterPose(monster, {
              state: leadWithHit ? 'hit' : 'dead',
            })
            monster.stateTimer = 0
            if (leadWithHit) {
              this.restartHitClip(monster)
            } else {
              monster.isDeadPending = false
            }
          } else if (monster.ownerId === myPlayerId) {
            const hitCommands: AiCommand[] =
              ai_handle_hit(
                monster.id,
                // 0 is never a real id (the server's counter starts at 1), so
                // it is the "no attacker" sentinel.
                monster.targetPlayerId ?? 0,
                !!monster.isLastHitSuccess,
                monster.pendingDamage ?? 0
              ) ?? []
            this.processAiCommands(monster, hitCommands)
          } else if (monster.isLastHitSuccess) {
            // Non-owner: show hit stagger visually; restart so a repeat hit
            // on the clamped clip doesn't no-op.
            this.applyMonsterPose(monster, { state: 'hit' })
            this.restartHitClip(monster)
            monster.stateTimer = 0
          } else if (monster.targetPlayerId && monster.state !== 'attack') {
            // Non-owner miss: show attack state visually
            this.applyMonsterPose(monster, { state: 'attack' })
            monster.stateTimer = 0
          }
        }
      }

      // Backstop: never leave a killed monster standing if the hit clip's
      // completion event is missed.
      if (monster.isDeadPending && !monster.impactDelay) {
        monster.deadPendingTimer = (monster.deadPendingTimer ?? 0) + deltaTime
        if (monster.deadPendingTimer > DEAD_PENDING_TIMEOUT_MS) {
          this.finishPendingDeath(monster)
        }
      }

      // Release the damage number once its attack-start delay has elapsed.
      if (monster.pendingDamageText) {
        monster.pendingDamageText.delay -= deltaTime
        if (monster.pendingDamageText.delay <= 0) {
          const { damage, hit } = monster.pendingDamageText
          monster.pendingDamageText = undefined
          this.emitDamageText(monster, damage, hit)
          damageTextFired = true
        }
      }

      // Only control monsters that YOU own
      if (monster.ownerId === myPlayerId) {
        // Guard: If dead or about to die, stop AI immediately
        if (monster.state === 'dead' || monster.isDeadPending) {
          this.monsters.set(monster.id, { ...monster })
          continue
        }

        const raw = ai_tick_brain(monster.id, deltaTime, nearbyPlayers)
        // ai_tick_brain returns a TickResult object with commands, position, rotation, state
        const result = raw as TickResult

        // Gate XZ movement here: the brain reports its internal state as attack
        // while chasing, then emits a Run Move command below; gating prevents
        // the intermediate attack snapshot from translating the model before
        // the Run command arrives.
        const resultPosition = result.position
          ? {
              x: result.position.x,
              y: this.monsterGroundY(
                monster,
                result.position.x,
                result.position.z
              ),
              z: result.position.z,
            }
          : undefined
        this.applyMonsterPose(
          monster,
          {
            position: resultPosition,
            rotation: result.rotation,
            state: result.state,
          },
          true
        )

        // Process transition commands (network sync, attacks)
        if (result.commands) {
          this.processAiCommands(monster, result.commands)
        }

        // Trigger reactivity with new reference
        this.monsters.set(monster.id, { ...monster })
      } else {
        // Interpolate remote monsters
        if (
          monster.state !== 'dead' &&
          !monster.isDeadPending &&
          this.isMovementState(monster.state) &&
          monster.targetPosition
        ) {
          this.moveTowards(monster, monster.targetPosition, deltaTime)
          this.monsters.set(monster.id, { ...monster })
        } else if (impactJustExpired || damageTextFired) {
          this.monsters.set(monster.id, { ...monster })
        }
      }
    }
  }

  private buildNearbyPlayers(gameState: GameState): Array<{
    id: number
    position: { x: number; y: number; z: number }
    health: number
  }> {
    const players: Array<{
      id: number
      position: { x: number; y: number; z: number }
      health: number
    }> = []

    // Current player
    if (gameState.currentPlayer) {
      players.push({
        id: gameState.currentPlayer.id,
        position: {
          x: gameState.currentPlayer.position.x,
          y: gameState.currentPlayer.position.y,
          z: gameState.currentPlayer.position.z,
        },
        health: gameState.currentPlayer.health ?? 0,
      })
    }

    // Remote players
    for (const [playerId, remoteState] of remotePlayerManager.players) {
      const remotePlayer = gameState.otherPlayers.get(playerId)
      players.push({
        id: playerId,
        position: remoteState.position,
        health: remotePlayer?.health ?? 0,
      })
    }

    return players
  }

  private updateMoveSpeedFromState(monster: MonsterData) {
    const def = getMonsterDef(monster.type)
    if (monster.state === 'run') {
      monster.moveSpeed = def?.runSpeed ?? 8
    } else if (monster.state === 'walk') {
      monster.moveSpeed = def?.walkSpeed ?? 1
    }
  }

  private isMovementState(state: MonsterData['state']) {
    return state === 'walk' || state === 'run'
  }

  private hasXzMovement(from: Position, to: Position) {
    return (
      Math.abs(from.x - to.x) > MONSTER_POSITION_EPSILON ||
      Math.abs(from.z - to.z) > MONSTER_POSITION_EPSILON
    )
  }

  private applyMonsterPose(
    monster: MonsterData,
    update: {
      position?: Position
      rotation?: number
      state?: MonsterState
      targetPosition?: Position
    },
    // The owner's brain reports its internal state as `attack` while chasing
    // and emits the locomotion (Run) Move command separately. Gating XZ
    // movement to walk/run states stops that intermediate attack snapshot from
    // sliding the model before the Run command arrives. Authoritative network
    // updates and visual-only state changes must NOT gate — they carry
    // ground-truth positions that have to be applied regardless of state.
    gateXzMovement = false
  ) {
    if (update.state) {
      // The frame the kill starts falling: XP held for it can ride the clip.
      const falling = update.state === 'dead' && monster.state !== 'dead'
      monster.state = update.state
      this.updateMoveSpeedFromState(monster)
      if (update.state === 'hit' || update.state === 'dead') {
        this.playPendingSwordHitSound(monster)
      }
      if (falling) releaseXpArrival(monster.id)
    }

    if (update.rotation !== undefined) {
      monster.rotation = update.rotation
    }

    if (update.targetPosition !== undefined) {
      monster.targetPosition = update.targetPosition
    }

    if (!update.position) return

    if (
      gateXzMovement &&
      !this.isMovementState(monster.state) &&
      this.hasXzMovement(monster.position, update.position)
    ) {
      // Non-movement states may still need terrain/deck height correction, but
      // XZ translation must go through walk/run so the rendered pose has a
      // locomotion animation to match it.
      monster.position = { ...monster.position, y: update.position.y }
      return
    }

    monster.position = update.position
  }

  private processAiCommands(monster: MonsterData, commands: AiCommand[]) {
    for (const cmd of commands) {
      if (cmd.type === 'Move') {
        const position = cmd.position
          ? (this.snapToMonsterGround(monster, cmd.position) ?? undefined)
          : undefined
        // Unsnappable destination: the server would reject the stale Y —
        // hold the report, the brain retries next tick.
        if (cmd.position && !position) continue
        const targetPosition = cmd.target_position
          ? (this.snapToMonsterGround(monster, cmd.target_position) ??
            cmd.target_position)
          : undefined

        this.applyMonsterPose(monster, {
          position,
          rotation: cmd.rotation,
          state: cmd.state,
          targetPosition,
        })
        networkManager.sendMonsterMove(
          cmd.monster_id,
          position ?? monster.position,
          cmd.rotation ?? monster.rotation,
          cmd.state ?? monster.state,
          targetPosition ?? monster.position
        )
      } else if (cmd.type === 'Attack' && cmd.target_player_id) {
        this.handleMonsterAttackStarted(cmd.monster_id)
        networkManager.sendMonsterAttack(cmd.monster_id, cmd.target_player_id)
      }
    }
  }

  updateMonsterFromNetwork(
    id: string,
    position: { x: number; y: number; z: number },
    rotation: number,
    state: MonsterData['state'],
    targetPosition: { x: number; y: number; z: number },
    ownerId?: number
  ) {
    const monster = this.monsters.get(id)
    if (monster) {
      // Guard: If monster is dead, don't allow state changes back to alive states
      if (monster.state === 'dead' && state !== 'dead') {
        return
      }

      // The fanout names the current owner; a mismatch means we missed a
      // handoff and would fight the real owner's stream with a stale brain.
      if (ownerId !== undefined && ownerId !== monster.ownerId) {
        const myPlayerId = get(gameStore).currentPlayer?.id
        if (monster.ownerId === myPlayerId) {
          ai_remove_brain(id)
        }
        monster.ownerId = ownerId
      }

      const hasPendingImpact =
        monster.impactDelay !== undefined && monster.impactDelay > 0
      const shouldDelayNetworkHit = hasPendingImpact && state === 'hit'

      const snappedPosition =
        this.snapToMonsterGround(monster, position) ?? position
      const snappedTargetPosition =
        this.snapToMonsterGround(monster, targetPosition) ?? targetPosition
      // Authoritative update: apply position/target directly (no movement gate).
      // When the hit is delayed, omit `state` so the current state is kept until
      // the pending impact resolves.
      this.applyMonsterPose(monster, {
        position: snappedPosition,
        rotation,
        state: shouldDelayNetworkHit ? undefined : state,
        targetPosition: snappedTargetPosition,
      })
      this.monsters.set(id, { ...monster })

      // Fanout skips the owner, so this is a correction — the brain must hear it
      // too or its next tick overwrites this pose. Unsnapped: the server's own
      // position is the authority, and emits get snapped anyway.
      if (
        monster.ownerId !== undefined &&
        monster.ownerId === get(gameStore).currentPlayer?.id
      ) {
        ai_apply_authoritative_position(id, position.x, position.y, position.z)
      }
    }
  }

  private moveTowards(
    monster: MonsterData,
    target: { x: number; y: number; z: number },
    deltaTime: number // in ms
  ): boolean {
    // Positions are canonical, so a step toward a target across the seam has
    // to take the periodic short path and stay canonical afterwards.
    const dx = shortestWrappedDeltaX(monster.position.x, target.x)
    const dz = target.z - monster.position.z
    const distance = Math.sqrt(dx * dx + dz * dz)

    const moveStep = (monster.moveSpeed * deltaTime) / 1000
    const onUpperFloor = (monster.currentFloor ?? 0) > 0
    // Dungeon floors live below Y=0, so the "stepped into water" guard
    // only applies to surface monsters.
    const inDungeon = (monster.floorLevel ?? 0) < 0

    if (distance <= moveStep) {
      const targetX = wrapWorldX(target.x)
      const y = onUpperFloor
        ? target.y
        : this.monsterGroundY(monster, targetX, target.z)
      if (!onUpperFloor && !inDungeon && y < 0) return true
      this.applyMonsterPose(monster, {
        position: { x: targetX, y, z: target.z },
      })
      return true
    } else {
      const newX = wrapWorldX(monster.position.x + (dx / distance) * moveStep)
      const newZ = monster.position.z + (dz / distance) * moveStep
      const y = onUpperFloor
        ? target.y
        : this.monsterGroundY(monster, newX, newZ)
      if (!onUpperFloor && !inDungeon && y < 0) return true
      this.applyMonsterPose(monster, {
        position: { x: newX, y, z: newZ },
      })
      return false
    }
  }
}

export const monsterManager = hmrSingleton(
  'monsterManager',
  () => new MonsterManager()
)

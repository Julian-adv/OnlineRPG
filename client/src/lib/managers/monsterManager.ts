import { SvelteMap } from 'svelte/reactivity'
import { hmrSingleton } from '../utils/hmr'
import * as THREE from 'three'
import { networkManager } from '../network/socket'
import { get } from 'svelte/store'
import { gameStore, type GameState } from '../stores/gameStore'
import { inventoryStore } from '../stores/inventoryStore'
import { remotePlayerManager } from './remotePlayerManager'
import type { MonsterData } from '../types/Monster'
import type { ServerMonster } from '../network/networkTypes'
import { getMonsterDef } from '../data/monsterDefs'
import { getItemDef, isRangedWeapon } from '../data/itemDefs'
import { flightMsFor, requestArrow } from '../stores/arrowStore'
import {
  getMaterialHitSoundUrl,
  getMaterialMissSoundUrl,
} from '../data/materialImpactSounds'
import { dungeonManager } from './dungeonManager'
import { housingManager } from './housingManager'
import { computeChaseAim } from './chase-aim'
import type { Position } from '../utils/movementUtils'
import type { TerrainHeightManager } from './terrainHeightManager'
import {
  playMonsterDeathSound,
  playBowSound,
  playSwordHitSound,
  playSwordMissSound,
} from './sfxManager'
import { clearXpArrival, releaseXpArrival } from './xpArrival'
import {
  shortestWrappedDeltaX,
  unwrapWorldXNear,
  wrapWorldX,
} from '../terrain/world-wrap'
import {
  PLAYER_ATTACK_DAMAGE_TEXT_DELAY_MS,
  DAMAGE_TEXT_AFTER_IMPACT_MS,
  PLAYER_RANGED_DRAW_DELAY_MS,
  PLAYER_RANGED_IMPACT_DELAY_MS,
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
// Server corrections under this size are absorbed by speed, not snapped.
const SYNC_BLEND_MAX_METERS = 2.5
const SYNC_CATCHUP_FRACTION = 0.5 // of move speed, while moving
const SYNC_IDLE_ABSORB_MPS = 2.0
// Engagement corrections (hit/attack) absorb fast: the model is already in a
// combat pose, so lingering meters from where it fights reads worse than a
// quick slide.
const SYNC_COMBAT_ABSORB_MPS = 5.0
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
  adoptOwnership(monster: ServerMonster) {
    const existing = this.monsters.get(monster.id)
    if (!existing) {
      this.spawnWithId(monster)
      return
    }
    existing.ownerId = monster.owner_id
    if (monster.floor_level !== undefined) {
      existing.floorLevel = monster.floor_level
    }
    this.monsters.set(monster.id, { ...existing })
    this.ensureBrain(existing, monster.aggressive)
  }

  spawnWithId(monster: ServerMonster) {
    if (this.monsters.has(monster.id)) return

    const type = monster.monster_type as MonsterData['type']
    // Corpses stay in the server's AOI, so re-entering a floor respawns
    // them dead; anything else transient collapses to idle.
    const spawnDead = monster.state === 'dead'
    const def = getMonsterDef(type)
    // Server is authoritative for HP and always sends health/max_health on
    // spawn; the constant is only a defensive fallback.
    const record: MonsterData = {
      id: monster.id,
      type,
      position: monster.position,
      rotation: 0,
      state: spawnDead ? 'dead' : 'idle',
      ownerId: monster.owner_id,
      moveSpeed: def?.walkSpeed ?? 1,
      stateTimer: 0,
      attackCounter: 0,
      hitCounter: 0,
      health: monster.health ?? 10,
      maxHealth: monster.max_health ?? 10,
      spawnPosition: { ...monster.position },
      floorLevel: monster.floor_level ?? 0,
    }
    this.monsters.set(monster.id, record)
    this.ensureBrain(record, monster.aggressive)
  }

  /** (Re)create our WASM brain from the monster's live state. Only monsters
   *  we own get one, and corpses never do. */
  private ensureBrain(monster: MonsterData, aggressive?: boolean) {
    if (monster.ownerId !== get(gameStore).currentPlayer?.id) return
    if (monster.state === 'dead') return
    ai_remove_brain(monster.id)
    this.ensureTemplatesLoaded()
    const def = getMonsterDef(monster.type)
    ai_create_brain({
      monsterId: monster.id,
      monsterType: monster.type,
      position: monster.position,
      health: monster.health,
      maxHealth: monster.maxHealth,
      walkSpeed: def?.walkSpeed ?? 1,
      runSpeed: def?.runSpeed ?? 8,
      attackRange: def?.attackRange ?? 2,
      chaseRange: def?.chaseRange ?? 25,
      attackCooldown: def?.attackCooldown ?? DEFAULT_MONSTER_ATTACK_COOLDOWN_MS,
      behavior: this.resolveBehavior(monster.type, aggressive),
      pathFloor: this.pathFloorFor(monster),
    })
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

    // Whose weapon this was. `PlayerMainHandChanged` broadcasts a remote
    // player's main hand and `gameStore` keeps it — it is what draws their
    // weapon on their model — so a remote shot resolves the same as a local
    // one, from the bag or from that.
    const state = get(gameStore)
    const isLocalPlayerAttack = playerId === state.currentPlayer?.id
    const weaponItemDefId = isLocalPlayerAttack
      ? get(inventoryStore).equipped.main_hand?.item_def_id
      : (state.otherPlayers.get(playerId)?.mainHand ?? undefined)
    const ranged = isRangedWeapon(weaponItemDefId)

    // A ranged shot resolves when its arrow lands, so everything the impact
    // drives waits out the flight on top of the release.
    const shooter = isLocalPlayerAttack
      ? state.currentPlayer?.position
      : remotePlayerManager.players.get(playerId)?.position
    const flightMs = ranged
      ? flightMsFor(
          Math.hypot(
            monster.position.x - (shooter?.x ?? monster.position.x),
            monster.position.z - (shooter?.z ?? monster.position.z)
          )
        )
      : 0
    monster.impactDelay = ranged
      ? PLAYER_RANGED_IMPACT_DELAY_MS + flightMs
      : PLAYER_ATTACK_IMPACT_DELAY_MS
    monster.targetPlayerId = playerId
    monster.isLastHitSuccess = hit
    const weaponMaterial = weaponItemDefId
      ? getItemDef(weaponItemDefId)?.material
      : undefined
    // A ranged hit stays silent at impact. The material table answers "what
    // does this weapon sound like striking that target", and a bow's `wood` is
    // its stave, not the arrow — with no rule for it the lookup falls through
    // to the default blade-on-leather thwack, which drowned the loose. The
    // release and the damage number carry the feedback until an arrow-impact
    // sound exists.
    if (hit && isLocalPlayerAttack && !ranged) {
      const monsterMaterial = getMonsterDef(monster.type)?.material
      monster.pendingSwordHitSoundUrl = getMaterialHitSoundUrl(
        weaponMaterial,
        monsterMaterial
      )
    } else {
      monster.pendingSwordHitSoundUrl = undefined
    }
    // For a bow the loose is the whoosh, and it plays on a hit too — at the
    // frame the arrow leaves, which is also when the shot resolves.
    if (isRangedWeapon(weaponItemDefId)) {
      playBowSound('draw', PLAYER_RANGED_DRAW_DELAY_MS)
      playBowSound('release', PLAYER_RANGED_IMPACT_DELAY_MS)
      // Asked for at the release frame so the scene reads the bow where the
      // draw ends. GameScene owns the launch: it has the player models.
      window.setTimeout(
        () => requestArrow({ playerId, monsterId, hit, flightMs }),
        PLAYER_RANGED_IMPACT_DELAY_MS
      )
    } else if (!hit) {
      // Every swing that misses whooshes, another player's included.
      playSwordMissSound(
        getMaterialMissSoundUrl(weaponMaterial),
        SWORD_MISS_DELAY_MS
      )
    }
    // Temporarily store damage to show at impact
    monster.pendingDamage = damage
    if (isLocalPlayerAttack) {
      monster.pendingDamageText = {
        delay: ranged
          ? PLAYER_RANGED_IMPACT_DELAY_MS + flightMs + DAMAGE_TEXT_AFTER_IMPACT_MS
          : PLAYER_ATTACK_DAMAGE_TEXT_DELAY_MS,
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

  // Facing is the client's call: the server's rotation lags the target.
  handleMonsterAttackStarted(
    monsterId: string,
    dedupeWindowMs = 0,
    target?: { x: number; z: number }
  ) {
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

    let rotation: number | undefined
    if (target) {
      const dx = shortestWrappedDeltaX(monster.position.x, target.x)
      const dz = target.z - monster.position.z
      if (dx !== 0 || dz !== 0) rotation = Math.atan2(dx, dz)
    }
    this.applyMonsterPose(monster, { rotation, state: 'attack' })
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
    // Built lazily: most clients own no monsters most frames.
    let nearbyMonsters:
      | ReturnType<MonsterManager['buildNearbyMonsters']>
      | undefined

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

        nearbyMonsters ??= this.buildNearbyMonsters()
        const raw = ai_tick_brain(
          monster.id,
          deltaTime,
          nearbyPlayers,
          nearbyMonsters
        )
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
        const blended = this.absorbSyncCorrection(monster, deltaTime)
        if (
          monster.state !== 'dead' &&
          !monster.isDeadPending &&
          this.isMovementState(monster.state) &&
          monster.targetPosition
        ) {
          const aim =
            this.liveChaseAim(monster, gameState) ?? monster.targetPosition
          this.moveTowards(monster, aim, deltaTime)
          this.monsters.set(monster.id, { ...monster })
        } else if (blended || impactJustExpired || damageTextFired) {
          this.monsters.set(monster.id, { ...monster })
        }
      }
    }
  }

  // Monster poses for cell separation (doc/MONSTER_SEPARATION.md); the
  // shared brain decides which states occupy cells, filters by its own
  // floor, and excludes itself.
  private buildNearbyMonsters(): Array<{
    id: string
    position: { x: number; y: number; z: number }
    state: string
    pathFloor: number
  }> {
    const list = []
    for (const m of this.monsters.values()) {
      if (m.state === 'dead' || m.isDeadPending || m.health <= 0) continue
      list.push({
        id: m.id,
        position: m.position,
        state: m.state,
        pathFloor: this.pathFloorFor(m),
      })
    }
    return list
  }

  // Dungeon monsters path on their depth's passability floor; surface
  // monsters use the open overworld (0).
  private pathFloorFor(monster: MonsterData): number {
    const fl = monster.floorLevel ?? 0
    return fl < 0 && dungeonManager.active
      ? dungeonManager.passabilityFloor(-fl)
      : 0
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
    ownerId?: number,
    chasing?: { player_id: number; stop_range: number } | null
  ) {
    const monster = this.monsters.get(id)
    if (monster) {
      // Guard: If monster is dead, don't allow state changes back to alive states
      if (monster.state === 'dead' && state !== 'dead') {
        return
      }
      monster.chaseAim = chasing
        ? { playerId: chasing.player_id, stopRange: chasing.stop_range }
        : undefined

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

      const jumpDx = shortestWrappedDeltaX(monster.position.x, position.x)
      const jumpDz = position.z - monster.position.z
      const jump = Math.hypot(jumpDx, jumpDz)
      const soften =
        jump > MONSTER_POSITION_EPSILON &&
        jump < SYNC_BLEND_MAX_METERS &&
        state !== 'dead' &&
        monster.state !== 'dead'
      monster.syncCorrection = soften ? { x: jumpDx, z: jumpDz } : undefined
      const snappedPosition = soften
        ? monster.position
        : (this.snapToMonsterGround(monster, position) ?? position)
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

  // A chasing monster walks at its target's live local position — exact for
  // the current player, interpolated for remotes — instead of the sync-old
  // leg target, so a head-on engagement starts where both actually stand.
  private liveChaseAim(
    monster: MonsterData,
    gameState: GameState
  ): { x: number; y: number; z: number } | undefined {
    const chase = monster.chaseAim
    const legTarget = monster.targetPosition
    if (!chase || !legTarget) return undefined
    const live =
      gameState.currentPlayer?.id === chase.playerId
        ? gameState.currentPlayer.position
        : remotePlayerManager.players.get(chase.playerId)?.position
    if (!live) return undefined
    const aim = computeChaseAim(
      monster.position,
      legTarget,
      live,
      chase.stopRange
    )
    if (!aim || aim === monster.position) return aim
    return housingManager.isMovementBlocked(
      monster.position.x,
      monster.position.z,
      unwrapWorldXNear(monster.position.x, aim.x),
      aim.z,
      this.pathFloorFor(monster),
      monster.position.y
    )
      ? undefined
      : aim
  }

  private absorbSyncCorrection(monster: MonsterData, deltaTime: number) {
    const c = monster.syncCorrection
    if (!c) return false
    const rate = this.isMovementState(monster.state)
      ? monster.moveSpeed * SYNC_CATCHUP_FRACTION
      : monster.state === 'hit' || monster.state === 'attack'
        ? SYNC_COMBAT_ABSORB_MPS
        : SYNC_IDLE_ABSORB_MPS
    const remaining = Math.hypot(c.x, c.z)
    const step = (rate * deltaTime) / 1000
    const p = monster.position
    const done = this.moveTowards(
      monster,
      { x: wrapWorldX(p.x + c.x), y: p.y, z: p.z + c.z },
      deltaTime,
      rate,
      false
    )
    if (done) {
      monster.syncCorrection = undefined
    } else {
      const keep = 1 - step / remaining
      c.x *= keep
      c.z *= keep
    }
    return true
  }

  // Server rotation arrives only per sync, so the step sets the heading;
  // a sync-correction nudge must not (`face`), it is not where it's going.
  private moveTowards(
    monster: MonsterData,
    target: { x: number; y: number; z: number },
    deltaTime: number, // in ms
    speed = monster.moveSpeed,
    face = true
  ): boolean {
    // Positions are canonical, so a step toward a target across the seam has
    // to take the periodic short path and stay canonical afterwards.
    const dx = shortestWrappedDeltaX(monster.position.x, target.x)
    const dz = target.z - monster.position.z
    const distance = Math.sqrt(dx * dx + dz * dz)
    if (face && distance > MONSTER_POSITION_EPSILON) {
      monster.rotation = Math.atan2(dx, dz)
    }

    const moveStep = (speed * deltaTime) / 1000
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

<script lang="ts">
  import { onMount } from 'svelte'
  import { useThrelte } from '@threlte/core'
  import * as THREE from 'three'
  import { gameStore, hoverTarget, type LocalPlayer } from '../stores/gameStore'
  import { networkManager } from '../network/socket'
  import type { PositionCorrection } from '../network/networkTypes'
  import { monsterManager } from '../managers/monsterManager'
  import { remotePlayerManager } from '../managers/remotePlayerManager'
  import { groundItemManager } from '../managers/groundItemManager'
  import { combatController } from '../managers/combatController'
  import {
    playPropSound,
    preloadFishingSounds,
    preloadPropSounds,
    preloadMonsterDeathSounds,
    preloadPlayerDeathSounds,
    preloadPlayerHurtSounds,
    preloadSwordHitSound,
    preloadSwordMissSound,
  } from '../managers/sfxManager'
  import {
    inputHandler,
    hoverTargetKey,
    type ClickIntent,
    type HoverTarget,
  } from '../managers/inputHandler'
  import { getNpcCapabilities } from '../data/traderDefs'
  import { tipHatManager } from '../managers/tipHatManager'
  import { tipHatDialog } from '../stores/tipHatStore'
  import { npcContextMenu, requestChatFocus } from '../stores/npcMenuStore'
  import {
    mapEditorMode,
    housingEditorMode,
    debugSpeedMode,
    torchLightEnabled,
    cameraRotationEnabled,
  } from '../stores/debugStore'
  import { localTorchEquipped, inventoryStore } from '../stores/inventoryStore'
  import { hungerState, SPRINT_MIN_SATIATION } from '../stores/hungerStore'
  import { getItemDef } from '../data/itemDefs'
  import {
    DEFAULT_MOVEMENT_CONFIG,
    SPRINT_SPEED_MULT,
    scaleMovementConfig,
    type Position,
    type MovementState,
    type MovementConfig,
    type PlayerState,
  } from '../utils/movementUtils'
  import type { TerrainHeightManager } from '../managers/terrainHeightManager'
  import {
    playerFloorOffset,
    playerVisualFloorLevel,
  } from '../stores/housingStore'
  import { currentDungeonDepth } from '../stores/dungeonStore'
  import { dungeonManager } from '../managers/dungeonManager'
  import { housingManager } from '../managers/housingManager'
  import { findPath } from '../managers/pathfinding'
  import { PROP_SWING_IMPACT_MS } from '../data/combatTiming'
  import {
    DUNGEON_DOOR_APPROACH,
    HOUSE_DOOR_APPROACH,
    NPC_TRADE_APPROACH,
    approachForInteraction,
    PICKUP_APPROACH,
    PROP_APPROACH,
    STALL_TRADE_APPROACH,
    TIP_HAT_APPROACH,
  } from '../data/approachRanges'
  import { passability_get_floor_at } from '../wasm/onlinerpg_shared'
  import { get } from 'svelte/store'
  import { sprintRequested } from '../stores/movementSettings'
  import { createPlayerPhysics } from './player-control/player-physics'
  import { subscribePlayerNetworkEvents } from './player-control/player-network-events'
  import type {
    PlayerControlEvent,
    PlayerControlUpdateOptions,
  } from './player-control/events'
  import {
    projectPlayerState,
    shouldEmitProjectedPlayerState,
  } from './player-control/fsm/projection'
  import {
    runMoveRequest,
    type MoveRequestActions,
  } from './player-control/fsm/move-request'
  import {
    createKeyboardMoveSender,
    createKeyboardSpeedRamp,
    createKeyboardTapTracker,
    runKeyboardFrame,
  } from './player-control/fsm/keyboard'
  import {
    dispatchPlayerControlEvent as dispatchQueuedPlayerControlEvent,
    createCanvasIntentEvent,
    type PlayerControlEventActions,
  } from './player-control/fsm/events'
  import { runPlayerMovementTick } from './player-control/fsm/movement-tick'
  import {
    beginJumpFeedback,
    shouldFinishJumpFeedback,
    transitionToDeadState,
    transitionToRespawnedState,
  } from './player-control/fsm/lifecycle'
  import {
    exitPickupInteraction as buildExitPickupInteraction,
    handlePickupGrab,
    applyObjectInteractionPosition,
    pickObjectExitPosition,
    beginPickupInteraction,
    beginObjectInteraction,
    exitObjectInteraction as buildExitObjectInteraction,
    handleInteractKey,
    getInteractionExitKind,
  } from './player-control/fsm/interaction'
  import {
    planApproach,
    resolveApproach,
    type ApproachSpec,
    type PendingApproach,
    type RouteQuality,
  } from './player-control/fsm/approach'

  import {
    beginAttack,
    ensureAttackState,
    transitionAttackToIdle,
    type ChaseMovement,
  } from './player-control/fsm/combat'
  import type { Pathing } from './player-control/fsm/movement-substrate'
  import {
    buildAttackState,
    buildInteractState,
  } from './player-control/player-state-builders'
  import type {
    MovingControlState,
    PickingUpControlState,
    PlayerControlStateName,
  } from './player-control/fsm/control-state'
  import { createLocalPlayerControlMachine } from './player-control/fsm/state-definitions'
  import { shortestWrappedDeltaX, wrapWorldX } from '../terrain/world-wrap'
  import {
    emoteRequest,
    emoteStopRequest,
    localEmoteAnim,
    EMOTE_ANIMS,
    HELD_EMOTE_ANIMS,
    SELF_ENDING_EMOTE_ANIMS,
  } from '../stores/emoteStore'
  import { respawnPoseRequest } from '../stores/respawnPoseStore'
  import { objectManager } from '../managers/objectManager'
  import { SitAnimationName } from '../types/animations'

  interface Props {
    onStateChange: (state: PlayerState) => void
    camera: THREE.Camera
    heightManager: TerrainHeightManager
    groundMeshes: THREE.Object3D[]
    groundItemMeshes: THREE.Object3D[]
    tipHatMeshes: THREE.Object3D[]
    stallMeshes: THREE.Object3D[]
    monsterMeshes: THREE.Group[]
    /** Invisible bind-pose boxes, one per monster — the 20 Hz hover raycast
     *  tests these instead of the skinned triangles. */
    monsterHoverMeshes: THREE.Group[]
    npcMeshes?: THREE.Object3D[]
    playerMeshes?: THREE.Object3D[]
    /** Invisible boxes, one per remote player, for the hover raycast. */
    playerHoverMeshes?: THREE.Object3D[]
    doorMeshes: THREE.Object3D[]
    objectMeshes: THREE.Object3D[]
    propMeshes: THREE.Object3D[]
    attackCooldown?: number
    /** Baked water surface height at a world XZ (for fishing cast detection). */
    waterSurfaceAt?: (x: number, z: number) => number
  }

  let {
    onStateChange,
    camera,
    heightManager,
    groundMeshes,
    groundItemMeshes,
    tipHatMeshes,
    stallMeshes,
    monsterMeshes,
    monsterHoverMeshes,
    npcMeshes = [],
    playerMeshes = [],
    playerHoverMeshes = [],
    doorMeshes,
    objectMeshes,
    propMeshes,
    attackCooldown,
    waterSurfaceAt,
  }: Props = $props()

  let floorOffset = 0
  playerFloorOffset.subscribe((v) => (floorOffset = v))

  let currentPlayer = $state<LocalPlayer | null>(null)

  /** Floor as broadcast to others. See `playerVisualFloorLevel`. */
  function wireFloorLevel(): number {
    const depth = get(currentDungeonDepth)
    return depth >= 1 ? -depth : get(playerVisualFloorLevel)
  }

  let lastSentFloorLevel: number | null = null

  /** Standalone floor send — move packets only land at waypoints. See
   * `ClientMessage::PlayerFloorChanged`. */
  function syncFloorLevel() {
    if (!currentPlayer) return
    const floorLevel = wireFloorLevel()
    if (floorLevel === lastSentFloorLevel) return
    lastSentFloorLevel = floorLevel
    networkManager.sendPlayerFloor(floorLevel)
  }
  playerVisualFloorLevel.subscribe(syncFloorLevel)
  currentDungeonDepth.subscribe(syncFloorLevel)

  const { renderer } = useThrelte()

  const physics = createPlayerPhysics({
    getHeightManager: () => heightManager,
    getCurrentPlayerY: () => currentPlayer?.position.y ?? null,
    getFloorOffset: () => floorOffset,
    getPassabilityFloor: currentPassabilityFloor,
  })
  const { sampleHeight, waypointHeight, isMovementBlocked, isUphillTooSteep } =
    physics

  // Movement data (target, integrator, A* waypoints, far-pickup target) lives
  // inside the machine's `moving` state — see movingState(). Leaving the moving
  // state drops that data, so there are no movement flags to reset here.
  // lastSentPosition is kinematic (send dedup), not state-membership data.
  let lastSentPosition = $state<Position | null>(null)

  // Use the same movement config as remote players, with debug speed multiplier.
  // The hunger multiplier mirrors the server's own movement sim (doc/HUNGER.md)
  // so prediction and authority agree.
  let speedMult = $derived(
    ($debugSpeedMode ? 10 : 1) * ($hungerState?.moveMult ?? 1)
  )
  let clickSprinting = false
  let startingClickMovement = false

  function sprintAvailable(): boolean {
    return ($hungerState?.satiation ?? 0) > SPRINT_MIN_SATIATION
  }

  function isSprintingNow(): boolean {
    if (!sprintAvailable()) return false
    const moving = playerControlMachine?.stateName === 'moving'
    // Combat chase runs (see getMovementMode) — at sprint speed, or a fleeing
    // monster outruns the player. Same satiation gate and cost as sprint.
    if (combatController.isInCombat && moving) return true
    if (clickSprinting && (startingClickMovement || moving)) return true
    return inputHandler.isSprintRequested && inputHandler.hasKeysPressed
  }

  // Called per frame — cache the scaled config so steady movement reuses one
  // object instead of allocating twice a frame.
  let cachedMoveMult = 1
  let cachedMoveConfig: MovementConfig = DEFAULT_MOVEMENT_CONFIG

  function movementConfig(): MovementConfig {
    const mult = speedMult * (isSprintingNow() ? SPRINT_SPEED_MULT : 1)
    if (mult !== cachedMoveMult) {
      cachedMoveMult = mult
      cachedMoveConfig = scaleMovementConfig(DEFAULT_MOVEMENT_CONFIG, mult)
    }
    return cachedMoveConfig
  }

  // Character rotation and current speed
  let playerRotation = $state(0)
  let currentSpeed = $state(0)

  const STAND_UP_DURATION = 300 // ms, matches animation crossfade duration
  let standUpTimer: ReturnType<typeof setTimeout> | null = null

  function clearStandUpTimer() {
    if (!standUpTimer) return
    clearTimeout(standUpTimer)
    standUpTimer = null
  }

  /** A seat exit waiting on the stand-up clip: where the player is heading
   *  (steers which side of the seat to step to) and what to do afterwards. */
  let pendingExit: {
    toward: Position | null
    then: (() => void) | null
  } | null = null

  const JUMP_FEEDBACK_DURATION_MS = 1500
  const JUMP_FEEDBACK_COOLDOWN_MS = 1000
  let jumpFeedbackTimer: ReturnType<typeof setTimeout> | null = null
  let lastJumpFeedbackAt = 0

  function clearJumpFeedbackTimer() {
    if (!jumpFeedbackTimer) return
    clearTimeout(jumpFeedbackTimer)
    jumpFeedbackTimer = null
  }

  // Prop-break swing: when the player reaches a clicked barrel/crate, swing the
  // sword once and break it at the contact frame, then drop back to idle after
  // the follow-through.
  const PROP_SWING_RETURN_MS = 1000
  let propSwingCounter = 0
  let propBreakTimer: ReturnType<typeof setTimeout> | null = null
  let propSwingIdleTimer: ReturnType<typeof setTimeout> | null = null

  function clearPropSwingTimers() {
    if (propBreakTimer) {
      clearTimeout(propBreakTimer)
      propBreakTimer = null
    }
    if (propSwingIdleTimer) {
      clearTimeout(propSwingIdleTimer)
      propSwingIdleTimer = null
    }
  }

  function enqueuePlayerControlEvent(event: PlayerControlEvent) {
    playerControlMachine.enqueueEvent(event)
  }

  /**
   * Briefly switch the player to the 'jump' state to play the jump animation
   * as a one-shot feedback that the terrain ahead is too steep. Cooldown
   * prevents the animation from restarting every frame while the user keeps
   * pushing into the slope.
   */
  function triggerJumpFeedback() {
    const transition = beginJumpFeedback({
      previousPlayerState: playerState,
      now: Date.now(),
      lastJumpFeedbackAt,
      cooldownMs: JUMP_FEEDBACK_COOLDOWN_MS,
    })
    lastJumpFeedbackAt = transition.runtime.lastJumpFeedbackAt
    if (transition.kind === 'cooldown') return

    setPlayerState(transition.nextPlayerState)
    transitionTo('jump_feedback')

    clearJumpFeedbackTimer()
    jumpFeedbackTimer = setTimeout(() => {
      jumpFeedbackTimer = null
      if (shouldFinishJumpFeedback(playerState)) {
        updatePlayerState()
        transitionTo('idle')
      }
    }, JUMP_FEEDBACK_DURATION_MS)
  }

  // Finish the in-flight pickup (settle the ground item) using the id owned by
  // the picking_up state. Callers always transition away from picking_up right
  // after, which drops the id — so this finishes exactly once per pickup. This
  // replaces the old reactive $effect backstop (L5): every path that leaves the
  // pickup state (stand-up via click/keyboard, anim finish, dead, respawn)
  // calls finishPendingPickup() explicitly.
  function finishPendingPickup() {
    const p = pickingUpState()
    if (p) groundItemManager.finishPickup(p.pendingPickupInstanceId)
  }

  function exitPickupInteraction() {
    const transition = buildExitPickupInteraction(playerState)
    if (transition.kind === 'ignored') return

    finishPendingPickup()
    setPlayerState(transition.nextPlayerState)
    transitionTo('idle')
  }

  function onInteractionFinished() {
    // A one-shot emote ends itself; notify so the server drops the stored
    // pose and remotes clear it. Held poses (bench, forge) stay until the
    // player moves, and pickup has its own exit below.
    if (playerState.state !== 'interact') return exitPickupInteraction()
    const anim = playerState.interactionAnim ?? ''
    if (SELF_ENDING_EMOTE_ANIMS.has(anim)) {
      exitObjectInteraction()
    } else if (anim === SitAnimationName.SIT_TO_STAND) {
      const exit = pendingExit
      pendingExit = null
      completeObjectExit(false, exit?.toward ?? undefined)
      exit?.then?.()
    } else {
      exitPickupInteraction()
    }
  }

  function onPickupGrab() {
    const p = pickingUpState()
    if (!p) return
    handlePickupGrab(p.pendingPickupInstanceId, {
      setInHand: (id) => groundItemManager.setInHand(id),
      remove: (id) => groundItemManager.remove(id),
      sendPickupItem: (id) => networkManager.sendPickupItem(id),
    })
  }

  /** Leave the current object/emote. A seated player first plays the
   *  stand-up clip; the real exit (and `then`) runs from
   *  onInteractionFinished when it ends. A rejected sit (notify=false) never
   *  sat, so it skips straight out. */
  function exitObjectInteraction(
    notify = true,
    then?: () => void,
    toward?: Position
  ) {
    const anim =
      playerState.state === 'interact' ? playerState.interactionAnim : undefined
    if (anim === SitAnimationName.SIT_TO_STAND) {
      pendingExit = { toward: toward ?? null, then: then ?? null }
      return
    }
    if (notify && anim === SitAnimationName.SIT) {
      setPlayerState(
        buildInteractState(
          playerState,
          playerState.position,
          playerState.rotation,
          SitAnimationName.SIT_TO_STAND,
          playerState.interactOffsetY ?? 0
        )
      )
      networkManager.sendStopInteraction()
      pendingExit = { toward: toward ?? null, then: then ?? null }
      return
    }
    completeObjectExit(notify, toward)
    then?.()
  }

  function completeObjectExit(notify: boolean, toward?: Position) {
    // Stepping out walks the player off the seat they were using. An emote
    // claims no object — it plays where the player stands — so every exit
    // path leaves an emote in place.
    const stepOut =
      playerState.state !== 'interact' ||
      !EMOTE_ANIMS.has(playerState.interactionAnim ?? '')
    if (stepOut && currentPlayer) {
      const seat = {
        x: currentPlayer.position.x,
        y: currentPlayer.position.y,
        z: currentPlayer.position.z,
      }
      applyObjectInteractionPosition(
        currentPlayer,
        pickObjectExitPosition(
          seat,
          playerRotation,
          (x, z) => isMovementBlocked(seat.x, seat.z, x, z, seat.y),
          toward
        ),
        {
          hasHeightData: (x, z) => heightManager.hasHeightData(x, z),
          sampleHeight,
        }
      )
    }

    setPlayerState(buildExitObjectInteraction(playerState))
    transitionTo('idle')

    if (notify) {
      networkManager.sendStopInteraction()
    }
  }

  function stopMovement() {
    const approach = movingState()?.approach ?? null
    clearStandUpTimer()
    currentSpeed = 0
    clickSprinting = false
    // Settle into idle BEFORE emitting: the projection derives 'moving' vs
    // 'idle' from the machine's owned state, so the transition must precede the
    // emit. Leaving the moving state also drops its target/movementState/path/
    // approach — nothing to reset. The walk-up action (or arrive()'s attack)
    // overrides idle right after.
    transitionTo('idle')
    updatePlayerState()
    // Every way a click-walk ends — arrival, a wall, a slope — lands here, so
    // this is the one place the walk-up action has to be resolved.
    if (
      approach &&
      currentPlayer &&
      currentPlayer.health > 0 &&
      resolveApproach(
        approach,
        currentPlayer.position,
        get(currentDungeonDepth)
      )
    ) {
      approach.act()
    }
  }

  // Explicitly drive the machine's owned state to a data-less state. The machine
  // no longer derives its state name from flags — callers transition at the real
  // decision points. Stateful transitions (moving/picking_up) carry their data.
  function transitionTo(
    name: Exclude<PlayerControlStateName, 'moving' | 'picking_up'>
  ) {
    playerControlMachine.transition({ name })
  }

  // `isMoving` is no longer a stored flag: being in motion IS being in the
  // moving/keyboard_moving state. Derive it from the machine's owned state.
  function isMovingNow(): boolean {
    const name = playerControlMachine.stateName
    return name === 'moving' || name === 'keyboard_moving'
  }

  // Narrowed views of the machine's owned state, for reading/mutating the data
  // the active state holds. Null when not in that state.
  function movingState(): MovingControlState | null {
    const s = playerControlMachine.state
    return s.name === 'moving' ? s : null
  }
  function pickingUpState(): PickingUpControlState | null {
    const s = playerControlMachine.state
    return s.name === 'picking_up' ? s : null
  }

  // Wrapper for sending move packets to track last sent position.
  // Wire format: dungeon depth d is floor_level -d; housing floors stay
  // 0..3 (client-internal -1 "outdoors" is clamped to 0).
  function sendPlayerMove(
    position: Position,
    rotation: number,
    append = false
  ) {
    const wrappedPosition = { ...position, x: wrapWorldX(position.x) }
    const floorLevel = wireFloorLevel()
    // The server checks the declared dungeon floor against Y: send the Y of
    // the floor we claim, whatever the caller sampled.
    if (floorLevel < 0) {
      const y = dungeonManager.floorHeightAt(
        -floorLevel,
        wrappedPosition.x,
        wrappedPosition.z
      )
      if (y !== null) wrappedPosition.y = y
    }
    lastSentPosition = wrappedPosition
    lastSentFloorLevel = floorLevel
    networkManager.sendPlayerMove(
      wrappedPosition,
      rotation,
      floorLevel,
      append,
      isSprintingNow()
    )
  }

  const keyboardMoveSender = createKeyboardMoveSender(sendPlayerMove)
  const keyboardTapTracker = createKeyboardTapTracker()
  const keyboardSpeedRamp = createKeyboardSpeedRamp()

  function writePlayerPosition(position: Position, rotation: number) {
    const wrappedX = wrapWorldX(position.x)
    gameStore.update((state) => {
      if (state.currentPlayer) {
        state.currentPlayer.position.set(wrappedX, position.y, position.z)
        state.currentPlayer.rotation = rotation
      }
      return state
    })
  }

  // The server refused a step, so we are somewhere it cannot follow. Snap to
  // its copy and drop the path that walked us out of sync — keeping it would
  // just march us back into the same refusal. Combat is left alone on purpose:
  // dropping the moving state drops the chase goal with it, so the next tick
  // re-routes from where we now actually are.
  function applyPositionCorrection(correction: PositionCorrection) {
    // The snap means we never reached what we were walking to.
    const m = movingState()
    if (m) m.approach = null
    stopMovement()
    playerRotation = correction.rotation
    writePlayerPosition(
      { x: correction.x, y: correction.y, z: correction.z },
      correction.rotation
    )
  }

  // Current player state
  let playerState = $state<PlayerState>({
    state: 'idle',
    speed: 0,
    rotation: 0,
    position: { x: 0, y: 0, z: 0 },
  })

  /** Turn to face a world point (rotation only; nothing is emitted). */
  function faceTowards(x: number, z: number) {
    if (!currentPlayer) return
    const dx = shortestWrappedDeltaX(currentPlayer.position.x, x)
    const dz = z - currentPlayer.position.z
    if (dx !== 0 || dz !== 0) playerRotation = Math.atan2(dx, dz)
  }

  // The panel highlight is a projection of the real state, not a flag set on
  // enter/exit: death, attacks, fishing, and bench-sitting all leave an emote
  // without passing any single exit function.
  let lastEmoteSync: string | null = null
  function syncLocalEmote(next: PlayerState) {
    const anim =
      next.state === 'interact' && EMOTE_ANIMS.has(next.interactionAnim ?? '')
        ? (next.interactionAnim ?? null)
        : null
    if (anim === lastEmoteSync) return
    lastEmoteSync = anim
    localEmoteAnim.set(anim)
  }

  function setPlayerState(next: PlayerState) {
    playerState = next
    onStateChange(next)
    syncLocalEmote(next)
  }

  gameStore.subscribe((state) => {
    const previousPlayerId = currentPlayer?.id ?? null
    currentPlayer = state.currentPlayer
    if (!currentPlayer) return

    const position = {
      x: currentPlayer.position.x,
      y: currentPlayer.position.y,
      z: currentPlayer.position.z,
    }
    if (currentPlayer.id === previousPlayerId) {
      playerState.position = position
      return
    }

    playerRotation = currentPlayer.rotation
    currentSpeed = 0
    setPlayerState({
      state: currentPlayer.health > 0 ? 'idle' : 'dead',
      speed: 0,
      rotation: currentPlayer.rotation,
      position,
    })
  })

  // Update player state and notify parent
  function updatePlayerState(totalDistance?: number) {
    const currentPosition = currentPlayer
      ? {
          x: currentPlayer.position.x,
          y: currentPlayer.position.y,
          z: currentPlayer.position.z,
        }
      : playerState.position

    const newState = projectPlayerState({
      currentPosition,
      isMoving: isMovingNow(),
      currentSpeed,
      playerRotation,
      totalDistance,
      hasTorch: $localTorchEquipped || $torchLightEnabled,
      isInCombat: combatController.isInCombat,
      attackCounter: combatController.attackCounter,
      isSprinting: isSprintingNow(),
    })

    // Only update if state actually changed
    if (shouldEmitProjectedPlayerState(playerState, newState)) {
      playerState = newState
      onStateChange(newState)
      syncLocalEmote(newState)
    }
  }

  /** Whether a wall stands between two points — the server's own gate on every
   *  blow. Also the movement tick's `attackLineBlocked`. */
  function attackLineBlocked(from: Position, to: Position, floor: number) {
    return housingManager.attackLineBlocked(from.x, from.z, to.x, to.z, floor)
  }

  /** Take the monster as a target and walk at it, attacking on arrival. */
  function chaseAndAttack(monsterId: string, goal: Position) {
    combatController.beginCombat(monsterId, false)
    handleClickToMove(goal)
  }

  // Initiate attack on a monster
  function initiateAttack(monsterId: string) {
    if (getInteractionExitKind(playerState) === 'pickup') {
      finishPendingPickup()
    }

    const monsterInfo = monsterManager.monsters.get(monsterId)

    // A wall between us refuses the blow server-side, so walk at the monster
    // instead of swinging into a rejection.
    if (
      monsterInfo &&
      currentPlayer &&
      attackLineBlocked(
        currentPlayer.position,
        monsterInfo.position,
        currentPassabilityFloor()
      )
    ) {
      chaseAndAttack(monsterId, monsterInfo.position)
      return
    }

    // Without this the first swing keeps the old facing until the next cycle.
    if (monsterInfo) {
      faceTowards(monsterInfo.position.x, monsterInfo.position.z)
    }

    const result = beginAttack({
      monsterId,
      monsterInfo,
      currentPosition: currentPlayer
        ? {
            x: currentPlayer.position.x,
            y: currentPlayer.position.y,
            z: currentPlayer.position.z,
          }
        : null,
      playerRotation,
      previousPlayerState: playerState,
      lastSentPosition,
      beginCombat: (id, inRange) => combatController.beginCombat(id, inRange),
      sendPlayerMove,
      sendPlayerAttack: (id) => networkManager.sendPlayerAttack(id),
    })

    if (result.kind === 'ignored_dead_target') return

    // Entering attacking drops any moving-state data (the chase that brought us
    // here), so there is nothing else to reset.
    setPlayerState(result.nextPlayerState)
    transitionTo('attacking')
  }

  // Transition from attack to idle state
  function transitionToIdle() {
    const transition = transitionAttackToIdle(playerState)
    if (transition.kind === 'ignored') return
    setPlayerState(transition.nextPlayerState)
    transitionTo('idle')
  }

  function transitionToDead() {
    const transition = transitionToDeadState(playerState)
    if (transition.kind === 'ignored_already_dead') return

    combatController.cancelCombat()
    inputHandler.clearTransientInput()
    currentSpeed = transition.runtime.currentSpeed
    // Finish any in-flight pickup while still in picking_up, before the dead
    // transition drops that state (L5: explicit finish on every pickup exit).
    finishPendingPickup()

    setPlayerState(transition.nextPlayerState)
    transitionTo('dead')
  }

  function transitionToRespawned() {
    if (!currentPlayer) return

    const transition = transitionToRespawnedState(playerState, {
      x: currentPlayer.position.x,
      y: currentPlayer.position.y,
      z: currentPlayer.position.z,
    })
    combatController.cancelCombat()
    inputHandler.clearTransientInput()
    clearStandUpTimer()
    pendingExit = null
    clearJumpFeedbackTimer()
    clearPropSwingTimers()
    currentSpeed = transition.runtime.currentSpeed
    playerRotation = transition.runtime.playerRotation
    finishPendingPickup()

    setPlayerState(transition.nextPlayerState)
    transitionTo('idle')
  }

  /** Check E key interaction (door toggle). Call from game loop. */
  function checkInteraction() {
    handleInteractKey({
      currentPlayer,
      consumeInteract: () => inputHandler.consumeInteract(),
      findNearestDoor: (x, z, y, range) =>
        housingManager.findNearestDoor(x, z, y, range),
      sendToggleDoor: (houseId, roomIndex, wallDir, segmentIndex) =>
        networkManager.sendToggleDoor(
          houseId,
          roomIndex,
          wallDir,
          segmentIndex
        ),
    })
  }

  // Stable action bags reused every frame by the movement/keyboard ticks.
  // They only read live `$state` inside their closures, so building them once
  // avoids reallocating ~20 closures per frame on the render hot path.
  const combatTickActions = {
    stopMovingToIdle: () => {
      if (isMovingNow()) {
        // Leaving the moving state drops its target/movementState. Transition
        // before emit so the projection sees idle (chase -> idle).
        transitionTo('idle')
        updatePlayerState()
      }
      transitionToIdle()
    },
    prepareReachedAttackRange: () => {
      currentSpeed = 0
      // Reached range stops movement (leaving moving drops its data); settle to
      // idle before the emit. beginAttack (next, same outcome) transitions to
      // attacking; if the target just died and beginAttack is ignored, we
      // correctly remain idle.
      transitionTo('idle')
      updatePlayerState()
    },
    beginAttack: initiateAttack,
    setChasingMovement: (chase: ChaseMovement) => {
      playerRotation = chase.playerRotation
      // Chase reports as 'moving' (playerState stays 'moving' while pathing to
      // the monster); 'attacking' is reserved for in-range swinging. Install the
      // freshly routed path on the live moving state, or — when chase resumes
      // from the attacking state — start a new moving state around it.
      const m = movingState()
      if (m) {
        m.target = chase.movementTarget
        m.movementState = chase.movementState
        m.waypoints = chase.pathWaypoints
        m.waypointIndex = 0
        m.chaseGoal = chase.chaseGoal
      } else {
        playerControlMachine.transition({
          name: 'moving',
          floor: chase.pathWaypoints[0].floor,
          target: chase.movementTarget,
          movementState: chase.movementState,
          waypoints: chase.pathWaypoints,
          waypointIndex: 0,
          chaseGoal: chase.chaseGoal,
          approach: null,
        })
      }
    },
    showAttackState: (nextRotation: number) => {
      playerRotation = nextRotation
      const transition = ensureAttackState(
        playerState,
        nextRotation,
        combatController.attackCounter
      )
      if (transition.kind === 'ignored') return
      setPlayerState(transition.nextPlayerState)
      transitionTo('attacking')
    },
    sendAttackCycle: (monsterId: string, nextRotation: number) => {
      playerRotation = nextRotation
      networkManager.sendPlayerAttack(monsterId)
      // Emit the attack state directly: the projection only knows idle/moving,
      // so it reported idle between swings.
      setPlayerState(
        buildAttackState(
          playerState,
          nextRotation,
          combatController.attackCounter
        )
      )
      transitionTo('attacking')
    },
  }

  // Hoisted like the action bags above: this is rebuilt every frame otherwise,
  // and `currentFloor` is a store read chase consumes at most ~1Hz.
  const chasePathing: Pathing = {
    get currentFloor() {
      return currentPassabilityFloor()
    },
    getFloorAt: getFloorAtForClick,
    findPath,
    waypointHeight,
  }

  const movementTickActions = {
    stopMovement,
    triggerJumpFeedback,
    setNextWaypoint: (
      nextCurrentSpeed: number,
      nextPlayerRotation: number,
      nextMovementTarget: Position,
      nextMovementState: MovementState,
      nextWaypointIndex: number
    ) => {
      currentSpeed = nextCurrentSpeed
      playerRotation = nextPlayerRotation
      const m = movingState()
      if (m) {
        m.target = nextMovementTarget
        m.movementState = nextMovementState
        m.waypointIndex = nextWaypointIndex
      }
    },
    arrive: (nextCurrentSpeed: number, nextPlayerRotation: number) => {
      currentSpeed = nextCurrentSpeed
      playerRotation = nextPlayerRotation
      // stopMovement() settles to idle (and emits) and runs any armed walk-up
      // action; the chase branch below overrides idle when arrival hands off to
      // an attack instead. A walk-up cancels combat, so only one can apply.
      stopMovement()

      if (combatController.isInCombat) {
        initiateAttack(combatController.targetMonsterId!)
      }
    },
    continueMovement: (
      nextCurrentSpeed: number,
      nextPlayerRotation: number,
      totalDistance: number
    ) => {
      currentSpeed = nextCurrentSpeed
      playerRotation = nextPlayerRotation
      updatePlayerState(totalDistance)
    },
  }

  const keyboardFrameActions = {
    exitPickupInteraction,
    exitObjectInteraction,
    clearClickMovement: () => {
      // Keyboard is taking the walk over, so the click's queued interaction is
      // off. The rest of the moving state needs no reset: keyboard transitions
      // to keyboard_moving (markMoving), idle (setKeyboardIdleRuntime), or via
      // stopMovement this same frame, all of which leave the moving state.
      const m = movingState()
      if (m) m.approach = null
    },
    cancelCombat: () => combatController.cancelCombat(),
    markMoving: () => {
      transitionTo('keyboard_moving')
    },
    setKeyboardIdleRuntime: () => {
      currentSpeed = 0
      transitionTo('idle')
    },
    emitKeyboardPlayerState: () => {
      updatePlayerState(isMovingNow() ? 100 : undefined)
    },
    stopMovement,
    triggerJumpFeedback,
    setMoved: (nextCurrentSpeed: number, nextPlayerRotation: number) => {
      currentSpeed = nextCurrentSpeed
      playerRotation = nextPlayerRotation
    },
    requestMove: (target: { x: number; z: number }) => {
      const tx = wrapWorldX(target.x)
      handleClickToMove({ x: tx, y: sampleHeight(tx, target.z), z: target.z })
    },
  }

  // Update player movement (click-to-move) with acceleration/deceleration
  function updatePlayerMovement(deltaTime: number) {
    const m = movingState()
    runPlayerMovementTick({
      deltaTime,
      currentPlayer,
      playerStateName: playerState.state,
      isMoving: isMovingNow(),
      currentSpeed,
      movementTarget: m?.target ?? null,
      movementState: m?.movementState ?? null,
      pathWaypoints: m?.waypoints ?? [],
      currentWaypointIndex: m?.waypointIndex ?? 0,
      chaseGoal: m?.chaseGoal ?? null,
      config: movementConfig(),
      isInCombat: combatController.isInCombat,
      combatController,
      cooldownMs:
        (attackCooldown ? attackCooldown * 1000 : 1500) /
        ($hungerState?.attackMult ?? 1),
      chasePathing,
      getMonsterInfo: (monsterId) => {
        const monsterData = monsterManager.monsters.get(monsterId)
        return monsterData
          ? {
              state: monsterData.state,
              isDeadPending: monsterData.isDeadPending,
            }
          : undefined
      },
      findMonsterPosition: (monsterId) =>
        monsterManager.findMeshPosition(monsterId, monsterMeshes),
      attackLineBlocked,
      sampleHeight,
      waypointHeight,
      hasHeightData: (x, z) => heightManager.hasHeightData(x, z),
      isMovementBlocked,
      isUphillTooSteep,
      setFloorLevel: (floor) => {
        const m = movingState()
        if (m) m.floor = floor
      },
      writePlayerPosition,
      sendPlayerMove,
      actions: {
        transitionToDead,
        transitionToRespawned,
        resetStoppedSpeed: () => {
          currentSpeed = 0
          updatePlayerState()
        },
        combat: combatTickActions,
        movement: movementTickActions,
      },
    })
  }

  function updateKeyboardMovement(deltaTime: number) {
    runKeyboardFrame({
      currentPlayer,
      hasKeysPressed: inputHandler.hasKeysPressed,
      isKeyboardMoving: playerControlMachine.stateName === 'keyboard_moving',
      interactionExit: getInteractionExitKind(playerState),
      hasMovementTarget: movingState() !== null,
      isInCombat: combatController.isInCombat,
      direction: inputHandler.getMovementDirection(),
      config: movementConfig(),
      deltaTimeSeconds: deltaTime / 1000,
      sampleHeight,
      isMovementBlocked,
      isUphillTooSteep,
      writePlayerPosition,
      moveSender: keyboardMoveSender,
      tapTracker: keyboardTapTracker,
      speedRamp: keyboardSpeedRamp,
      actions: keyboardFrameActions,
    })
  }

  function createMoveRequestActions(
    clickPosition: Position,
    options: { approach?: PendingApproach | null }
  ): MoveRequestActions {
    return {
      exitPickupAndRetry: () => {
        exitPickupInteraction()
        handleClickToMove(clickPosition, options)
      },
      exitObjectAndDelay: () => {
        exitObjectInteraction(
          true,
          () => {
            clearStandUpTimer()
            standUpTimer = setTimeout(() => {
              standUpTimer = null
              enqueuePlayerControlEvent({
                type: 'delayed_request_move',
                position: { ...clickPosition },
                approach: options.approach ?? null,
              })
            }, STAND_UP_DURATION)
          },
          clickPosition
        )
      },
      applyStartedMovement: (started) => {
        playerRotation = started.playerRotation
        // The moving state OWNS the path data. Transition before emit: the
        // projection derives 'moving' from the machine's owned state.
        playerControlMachine.transition({
          name: 'moving',
          floor: started.pathWaypoints[0].floor,
          target: started.movementTarget,
          movementState: started.movementState,
          waypoints: started.pathWaypoints,
          waypointIndex: started.currentWaypointIndex,
          chaseGoal: null,
          // Armed only now: a refused request (dead, keyboard held) must not
          // leave an action waiting to fire on some later, unrelated stop.
          approach: options.approach ?? null,
        })
        updatePlayerState(started.movementState.totalDistance)
      },
    }
  }

  /** Passability floor for path queries: dungeon depths map to 4+. On the
   * surface: the moving leg's floor, else the floor the player stands on.
   * `moving.floor` may carry raw dungeon waypoint floors (4+) — clamped out
   * here, the single read site, so they can't outlive a surfacing. */
  function currentPassabilityFloor(): number {
    const depth = get(currentDungeonDepth)
    if (depth >= 1) {
      // On a stair shaft this resolves to the shaft's lower floor (see
      // dungeonManager.startFloorAt) so a path to the surface climbs out
      // instead of routing back down to the bottom landing.
      return currentPlayer
        ? dungeonManager.startFloorAt(
            currentPlayer.position.x,
            currentPlayer.position.z,
            currentPlayer.position.y
          )
        : dungeonManager.passabilityFloor(depth)
    }
    const legFloor = movingState()?.floor
    return legFloor !== undefined &&
      legFloor < dungeonManager.consts.floorIndexBase
      ? legFloor
      : get(playerVisualFloorLevel)
  }

  /**
   * Floor lookup for click targets. The dungeon grids cover the whole
   * footprint at every depth, so a click that lands nearer a dungeon floor's Y
   * than the surface resolves to that floor. Re-weigh the surface (floor 0 at
   * the entrance Y) as a candidate: if the click sits at least as close to the
   * surface, treat it as the surface. This is what lets an upper-landing click
   * while standing mid-stairs (depth ≥ 1) target floor 0 so the path climbs
   * out instead of routing down to the bottom landing first.
   */
  function getFloorAtForClick(x: number, z: number, y: number): number {
    const depth = get(currentDungeonDepth)
    // Stairwell clicks resolve via the shaft mapping, not the raw Y lookup:
    // intermediate steps are keyed to the shallower connected floor, so the
    // Y-based lookup returns the deeper floor and strands A* at the bottom
    // landing — the player walks all the way down, then climbs back to the
    // clicked step. Underground, query the current depth's shafts. On the
    // surface (depth 0) the only clickable shaft is the entrance stairs
    // (floor 1's up-shaft), so query it at depth 1: a mid-stair click then
    // targets floor 0 and the player stops right at the clicked step.
    if (depth >= 1 || dungeonManager.isOnEntranceShaft(x, z)) {
      const shaftFloor = dungeonManager.shaftPathfindingFloorAt(
        x,
        z,
        y,
        Math.max(depth, 1)
      )
      if (shaftFloor !== null) return shaftFloor
    }

    const floor = passability_get_floor_at(x, z, y)
    const fib = dungeonManager.consts.floorIndexBase
    if (floor < fib) return floor
    const ent = dungeonManager.entrancePos
    if (!ent) return depth < 1 ? 0 : floor
    // Target the floor that is currently SHOWN to the player, independent of
    // logical depth: when underground (depth ≥ 1) the dungeon floor is what's
    // rendered, so a click targets it. Otherwise, classify by the CLICK target,
    // not the player: a click on the entrance shaft is a descent, but a click on
    // the open surface — even while standing on the top landing, which still
    // counts as "on the shaft" — must fall through to the surface-vs-floor Y
    // heuristic so the player isn't routed back down into the dungeon.
    const inDungeonView = depth >= 1 || dungeonManager.isOnEntranceShaft(x, z)
    if (inDungeonView) return floor
    const depthOfFloor = floor - fib + 1
    const surfaceDist = Math.abs(y - ent.y)
    const floorDist = Math.abs(y - dungeonManager.floorY(depthOfFloor))
    return surfaceDist <= floorDist ? 0 : floor
  }

  function handleClickToMove(
    clickPosition: Position,
    options: { approach?: PendingApproach | null; sprinting?: boolean } = {}
  ) {
    // Any fresh movement cancels a pending prop break/open (breakProp/openProp
    // re-arm it after their own walk-up call below).
    dungeonManager.clearPendingBreak()
    dungeonManager.clearPendingOpen()
    // Approach moves (chase, walk-up) carry no modifier: follow the preference.
    clickSprinting =
      (options.sprinting ?? sprintRequested(false)) && sprintAvailable()

    // Start A* from the player's current passability floor — on a stair shaft
    // that is the shaft's keyed (lower) floor (see currentPassabilityFloor /
    // dungeonManager.startFloorAt), which differs from the clicked room's floor, so
    // the search traverses the stairs instead of being confined to one floor.
    startingClickMovement = true
    runMoveRequest({
      clickPosition,
      currentPlayer,
      interactionExit: getInteractionExitKind(playerState),
      isMoving: isMovingNow(),
      hasKeyboardInput: inputHandler.hasKeysPressed,
      currentFloor: currentPassabilityFloor(),
      getFloorAt: getFloorAtForClick,
      findPath,
      waypointHeight,
      sendPlayerMove,
      startSpeed: currentSpeed,
      actions: createMoveRequestActions(clickPosition, options),
    })
    startingClickMovement = false
    if (playerControlMachine.stateName !== 'moving') clickSprinting = false
  }

  /** `claim` is false when the server already holds the object for us. */
  function enterInteraction(
    intent: Extract<ClickIntent, { type: 'interact_object' }>,
    claim = true
  ) {
    if (getInteractionExitKind(playerState) === 'pickup') {
      finishPendingPickup()
    }

    // Re-sitting mid stand-up must not let the old exit's continuation fire.
    pendingExit = null

    const result = beginObjectInteraction({
      intent,
      previousPlayerState: playerState,
      cancelCombat: () => combatController.cancelCombat(),
    })

    // Entering object_interacting drops any moving data; just face the object.
    playerRotation = result.playerRotation
    setPlayerState(result.nextPlayerState)
    transitionTo('object_interacting')

    if (currentPlayer) {
      applyObjectInteractionPosition(currentPlayer, result.entryPosition, {
        hasHeightData: (x, z) => heightManager.hasHeightData(x, z),
        sampleHeight,
      })
    }

    if (claim) {
      networkManager.sendInteractObject(intent.objectType, intent.objectId)
    }
  }

  /** Lie down on the bed the server respawned us on. */
  async function enterRespawnPose(objectType: string) {
    if (!currentPlayer) return
    const { x, z } = currentPlayer.position
    const { anim, interactOffset, placement, rotation } =
      await objectManager.resolvePose(objectType, x, z)
    if (!placement || rotation === undefined || !currentPlayer) return
    enterInteraction(
      {
        type: 'interact_object',
        objectId: placement.id,
        objectType,
        interaction: anim,
        position: { x: placement.x, y: placement.y, z: placement.z },
        rotation,
        interactOffset,
      },
      false
    )
  }

  /** Enter an emote clip in place. Unlike enterInteraction there is no object
   *  to face, snap to, or claim, and the server already heard about it through
   *  the chat command — so no sendInteractObject here. */
  function startEmote(anim: string) {
    if (!currentPlayer) return
    if (getInteractionExitKind(playerState) === 'pickup') {
      finishPendingPickup()
    }

    // Leftover deceleration would let the movement tick's resetStoppedSpeed
    // project the fresh interact state back to idle one frame later.
    currentSpeed = 0
    pendingExit = null

    const result = beginObjectInteraction({
      intent: {
        type: 'interact_object',
        objectId: 0,
        objectType: anim,
        interaction: anim,
        position: {
          x: currentPlayer.position.x,
          y: currentPlayer.position.y,
          z: currentPlayer.position.z,
        },
        rotation: playerRotation,
      },
      previousPlayerState: playerState,
      cancelCombat: () => combatController.cancelCombat(),
    })

    setPlayerState(result.nextPlayerState)
    transitionTo('object_interacting')
  }

  $effect(() => {
    const anim = $emoteRequest
    if (!anim) return
    emoteRequest.set(null)
    startEmote(anim)
  })

  $effect(() => {
    const objectType = $respawnPoseRequest
    if (!objectType) return
    respawnPoseRequest.set(null)
    void enterRespawnPose(objectType)
  })

  $effect(() => {
    if (!$emoteStopRequest) return
    emoteStopRequest.set(false)
    // Only a performance ends here — the tune running out or Escape. By now
    // the player may have sat down on something, and that pose is not ours
    // to cancel.
    if (
      playerState.state === 'interact' &&
      HELD_EMOTE_ANIMS.has(playerState.interactionAnim ?? '')
    ) {
      exitObjectInteraction()
    }
  })

  function enterPickup(instanceId: number) {
    // Face the item: an in-reach click never walks, and a blocked walk-up
    // stops facing its travel direction.
    const item = groundItemManager.items.get(instanceId)
    if (item) faceTowards(item.position.x, item.position.z)

    const result = beginPickupInteraction({
      instanceId,
      previousPlayerState: { ...playerState, rotation: playerRotation },
      hasGroundItem: () => item !== undefined,
      beginPickup: (id) => groundItemManager.beginPickup(id),
      cancelCombat: () => combatController.cancelCombat(),
    })

    if (result.kind === 'ignored') return

    // The picking_up state OWNS the instance id being grabbed; entering it drops
    // any moving data (the far-pickup approach that led here).
    currentSpeed = 0
    if (currentPlayer) sendPlayerMove(currentPlayer.position, playerRotation) // others see the facing
    networkManager.sendPickupStarted()
    setPlayerState(result.nextPlayerState)
    playerControlMachine.transition({
      name: 'picking_up',
      pendingPickupInstanceId: result.pendingPickupInstanceId,
    })
  }

  /** How A* rates a walk-up goal, so the plan can pick a goal the player can
   *  actually get to. Deliberately re-runs the search the move itself will make
   *  — same start, same floors — so the two agree on what is reachable; a click
   *  is a rare enough event to pay for it twice. */
  function routeQuality(target: Position): RouteQuality {
    if (!currentPlayer) return 'none'
    const result = findPath(
      currentPlayer.position.x,
      currentPlayer.position.z,
      currentPassabilityFloor(),
      target.x,
      target.z,
      getFloorAtForClick(target.x, target.z, target.y)
    )
    if (result.found) return 'found'
    return result.waypoints.length > 0 ? 'partial' : 'none'
  }

  /** `canActNow` is false while an interaction animation still has to be
   *  exited — the walk-up runs the exit, then fires the action on arrival. */
  function approachAndAct(
    spec: ApproachSpec,
    act: () => void,
    canActNow = true
  ) {
    if (!currentPlayer || currentPlayer.health <= 0) return

    const plan = planApproach(
      currentPlayer.position,
      spec,
      routeQuality,
      canActNow
    )
    if (plan.kind === 'unreachable') return
    if (plan.kind === 'act_now') {
      act()
      return
    }

    combatController.cancelCombat()
    handleClickToMove(plan.target, {
      approach: { spec, depth: get(currentDungeonDepth), act },
    })
  }

  function pickupItem(
    intent: Extract<ClickIntent, { type: 'pickup_ground_item' }>
  ) {
    if (playerState.state === 'dead') return
    const item = groundItemManager.items.get(intent.instanceId)
    // Never pick up straight from an interaction: re-entering picking_up would
    // overwrite the owned id and strand the grabbed item on the hand bone
    // (finishPickup never runs). The walk-up settles the interaction first.
    approachAndAct(
      { position: item?.position ?? intent.position, ...PICKUP_APPROACH },
      () => enterPickup(intent.instanceId),
      getInteractionExitKind(playerState) === 'none'
    )
  }

  function interactObject(
    intent: Extract<ClickIntent, { type: 'interact_object' }>
  ) {
    approachAndAct(
      {
        position: intent.position,
        ...approachForInteraction(intent.interaction),
      },
      () => enterInteraction(intent)
    )
  }

  function toggleDoor(intent: Extract<ClickIntent, { type: 'toggle_door' }>) {
    approachAndAct({ position: intent.position, ...HOUSE_DOOR_APPROACH }, () =>
      networkManager.sendToggleDoor(
        intent.houseId,
        intent.roomIndex,
        intent.wallDir,
        intent.segmentIndex
      )
    )
  }

  function toggleDungeonDoor(
    intent: Extract<ClickIntent, { type: 'toggle_dungeon_door' }>
  ) {
    approachAndAct(
      { position: intent.position, ...DUNGEON_DOOR_APPROACH },
      () => {
        const id = dungeonManager.dungeonId
        if (id) {
          networkManager.sendToggleDungeonDoor(id, intent.depth, intent.doorId)
        }
      }
    )
  }

  function tradeWithNpc(
    intent: Extract<ClickIntent, { type: 'interact_npc' }>
  ) {
    approachAndAct({ position: intent.position, ...NPC_TRADE_APPROACH }, () =>
      networkManager.sendOpenShop(intent.playerId)
    )
  }

  function tipHat(intent: Extract<ClickIntent, { type: 'tip_hat' }>) {
    approachAndAct({ position: intent.position, ...TIP_HAT_APPROACH }, () => {
      const hat = tipHatManager.hats.get(intent.hatId)
      if (hat) tipHatDialog.set({ hatId: hat.id, ownerName: hat.owner_name })
    })
  }

  /** The stall standing there is its owner's consent, so there is no request. */
  function tradeAtStall(intent: Extract<ClickIntent, { type: 'stall' }>) {
    approachAndAct({ position: intent.position, ...STALL_TRADE_APPROACH }, () =>
      networkManager.sendPlayerTradeAtStall(intent.stallId)
    )
  }

  /** Shared walk-up for a clicked interactive prop: move to within reach if
   *  needed (it's a solid pillar, so stop just short), then arm `setPending` so
   *  the dungeon layer fires the break/open once the player is in range. */
  function approachProp(
    intent: { depth: number; propId: number; position: Position },
    setPending: (p: {
      depth: number
      propId: number
      x: number
      z: number
    }) => void
  ) {
    if (!currentPlayer) return
    const plan = planApproach(
      currentPlayer.position,
      { position: intent.position, ...PROP_APPROACH },
      routeQuality
    )
    if (plan.kind === 'unreachable') return
    if (plan.kind === 'walk') {
      combatController.cancelCombat()
      handleClickToMove(plan.target)
    }
    setPending({
      depth: intent.depth,
      propId: intent.propId,
      x: intent.position.x,
      z: intent.position.z,
    })
  }

  /** Click a barrel/crate: walk up, then arm the break. The dungeon layer fires
   *  it via the server once the player is in range. */
  function breakProp(intent: Extract<ClickIntent, { type: 'break_prop' }>) {
    approachProp(intent, (p) => dungeonManager.setPendingBreak(p))
  }

  /** Click a chest: walk up, then arm the open. The dungeon layer sends the open
   *  via the server once in range; every client (the opener included) plays the
   *  lid animation on the broadcast. */
  function openProp(intent: Extract<ClickIntent, { type: 'open_prop' }>) {
    // Already open — nothing to do (avoid a pointless walk-up).
    if (dungeonManager.isPropOpened(intent.depth, intent.propId)) return
    approachProp(intent, (p) => dungeonManager.setPendingOpen(p))
  }

  /** The player has walked up to a clicked barrel/crate: swing the sword once
   *  and break it at the contact frame. Called from the dungeon layer the frame
   *  the player comes into range (see GameSceneDungeonLayer onPropReady). */
  export function swingAndBreakProp(
    entranceId: string,
    depth: number,
    propId: number,
    x: number,
    z: number
  ) {
    if (!currentPlayer) return
    // Don't interrupt an in-flight swing (the layer can fire across frames).
    if (playerState.state === 'attack' && propBreakTimer) return
    combatController.cancelCombat()
    clearPropSwingTimers()

    // Face the prop, stop, and play one slash. State 'attack' selects the slash
    // clip; a changed attackCounter re-triggers it (our own counter since this
    // swing isn't combat-driven). currentSpeed 0 keeps the movement tick from
    // projecting the state back to idle while we hold the swing.
    faceTowards(x, z)
    currentSpeed = 0
    propSwingCounter += 1
    setPlayerState(
      buildAttackState(playerState, playerRotation, propSwingCounter)
    )
    transitionTo('attacking')
    sendPlayerMove(currentPlayer.position, playerRotation) // others see the facing

    propBreakTimer = setTimeout(() => {
      propBreakTimer = null
      playPropSound('break')
      dungeonManager.noteSelfBreak(depth, propId)
      networkManager.sendBreakDungeonProp(entranceId, depth, propId)
    }, PROP_SWING_IMPACT_MS)
    propSwingIdleTimer = setTimeout(() => {
      propSwingIdleTimer = null
      if (playerState.state === 'attack') transitionToIdle()
    }, PROP_SWING_RETURN_MS)
  }

  // Sticky hover keeps the target ring up while the pointer sits in the
  // hovered monster's margin; a click there should attack, not walk, even
  // though the ray misses the actual silhouette.
  function hoveredMonsterAttackIntent(): ClickIntent | null {
    const hover = get(hoverTarget)
    if (hover?.kind !== 'monster' || isMonsterDead(hover.monsterId)) return null
    const monster = monsterManager.monsters.get(hover.monsterId)
    if (!monster || !currentPlayer) return null
    const p = currentPlayer.position
    const dx = shortestWrappedDeltaX(p.x, monster.position.x)
    const dz = monster.position.z - p.z
    return {
      type: 'attack_monster',
      monsterId: hover.monsterId,
      hitPoint: { x: p.x + dx, y: monster.position.y, z: monster.position.z },
      distance: Math.sqrt(dx * dx + dz * dz),
    }
  }

  // Same coherence for NPCs: a click in the sticky margin interacts instead
  // of walking. Non-NPC players stay left-click inert by design, so hovering
  // one never overrides a click.
  function hoveredNpcInteractIntent(): ClickIntent | null {
    const hover = get(hoverTarget)
    if (hover?.kind !== 'player' || !currentPlayer) return null
    if (!get(gameStore).otherPlayers.get(hover.playerId)?.isOfficialNpc)
      return null
    const npcPos = remotePlayerManager.players.get(hover.playerId)?.position
    if (!npcPos) return null
    const p = currentPlayer.position
    return {
      type: 'interact_npc',
      playerId: hover.playerId,
      position: {
        x: p.x + shortestWrappedDeltaX(p.x, npcPos.x),
        y: npcPos.y,
        z: npcPos.z,
      },
    }
  }

  function processClickIntent(event: MouseEvent): ClickIntent {
    const intent = inputHandler.processCanvasClick(event, {
      camera,
      monsterMeshes,
      npcMeshes,
      doorMeshes,
      objectMeshes,
      propMeshes,
      groundItemMeshes,
      tipHatMeshes,
      stallMeshes,
      groundMeshes,
      playerPosition: {
        x: currentPlayer!.position.x,
        y: currentPlayer!.position.y,
        z: currentPlayer!.position.z,
      },
      playerVisualFloorLevel: get(playerVisualFloorLevel),
      isMonsterDead,
      canCastFishing:
        getItemDef(get(inventoryStore).equipped.main_hand?.item_def_id ?? '')
          ?.category === 'fishing_rod' && currentPassabilityFloor() === 0,
      waterSurfaceAt,
    })
    if (intent.type === 'move_to_ground' || intent.type === 'none') {
      return (
        hoveredMonsterAttackIntent() ?? hoveredNpcInteractIntent() ?? intent
      )
    }
    return intent
  }

  /** Right-click on an NPC: open the context menu with the interactions the
   *  NPC's data supports (doc/ECONOMY.md "거래 진입 UI"). Right-click on a
   *  player offers to report the picture on their cape, which is the only
   *  brake on what people print (doc/CAPE_CUSTOMIZATION.md). */
  function handleNpcContextMenu(event: MouseEvent) {
    if (!currentPlayer || currentPlayer.health <= 0) return
    const intent = processClickIntent(event)
    if (intent.type === 'interact_npc') {
      const npc = get(gameStore).otherPlayers.get(intent.playerId)
      if (npc?.isOfficialNpc) {
        const caps = getNpcCapabilities(npc.name)
        const entries = [{ label: 'Talk', action: () => requestChatFocus() }]
        if (caps.trade) {
          entries.push({
            label: 'Trade',
            action: () => tradeWithNpc(intent),
          })
        }
        npcContextMenu.set({
          npcName: npc.name,
          screenX: event.clientX,
          screenY: event.clientY,
          entries,
        })
        return
      }
    }

    const playerId = inputHandler.pickPlayer(event, camera, playerMeshes)
    if (playerId === null) return
    const player = get(gameStore).otherPlayers.get(playerId)
    if (!player || player.isOfficialNpc || !player.backTexture) return
    npcContextMenu.set({
      npcName: player.name,
      screenX: event.clientX,
      screenY: event.clientY,
      entries: [
        {
          label: 'Report cape',
          action: () => networkManager.sendReportCapeTexture(playerId),
        },
      ],
    })
  }

  function handleCanvasClickIntent(event: MouseEvent) {
    if (event.button === 0 && $cameraRotationEnabled) return
    const editorMode = $mapEditorMode || $housingEditorMode
    if (event.button === 2 && !editorMode) {
      handleNpcContextMenu(event)
      return
    }
    const playerControlEvent = createCanvasIntentEvent({
      event,
      editorMode,
      currentPlayer,
      processIntent: () => processClickIntent(event),
    })
    if (!playerControlEvent) return

    enqueuePlayerControlEvent(playerControlEvent)
  }

  function createPlayerControlEventActions(): PlayerControlEventActions {
    return {
      attackInRange: (monsterId) => {
        // initiateAttack transitions to attacking, which drops any moving data
        // (no separate runtime reset needed).
        initiateAttack(monsterId)
      },
      chaseAndAttack,
      toggleDoor,
      toggleDungeonDoor,
      interactObject,
      pickupItem,
      interactNpc: (intent) => {
        const npc = get(gameStore).otherPlayers.get(intent.playerId)
        if (!npc?.isOfficialNpc) return
        // Click default per NPC kind: merchants open their shop, everyone
        // else starts a conversation. Right-click offers both explicitly.
        const caps = getNpcCapabilities(npc.name)
        if (caps.defaultAction === 'trade') {
          tradeWithNpc(intent)
        } else {
          requestChatFocus()
        }
      },
      tipHat,
      tradeAtStall,
      breakProp,
      openProp,
      moveToGround: (position, sprinting) => {
        combatController.cancelCombat()
        const snapped = dungeonManager.snapDescentWallClick(
          position.x,
          position.z,
          position.y
        )
        handleClickToMove(snapped ?? position, { sprinting })
      },
      castFishing: (intent) => {
        if (!currentPlayer || currentPlayer.health <= 0) return
        // Stop and face the water before the cast — the server aborts a
        // session on any movement, so a cast while pathing would cancel
        // itself on the next waypoint send.
        combatController.cancelCombat()
        stopMovement()
        faceTowards(intent.position.x, intent.position.z)
        // Commit the facing to the rendered state too, or the model keeps its
        // old rotation and casts over its shoulder.
        setPlayerState({ ...playerState, rotation: playerRotation })
        // Updates the server-stored rotation (late joiners); live bystanders
        // get the facing from FishingCasted itself.
        sendPlayerMove(currentPlayer.position, playerRotation)
        networkManager.sendFishingCast(intent.position)
      },
      requestMove: handleClickToMove,
      onInteractionFinished,
      onPickupGrab,
      onInteractionRejected: () => {
        if (playerState.state === 'interact') exitObjectInteraction(false)
      },
    }
  }

  function dispatchPlayerControlEvent(event: PlayerControlEvent) {
    // A fresh click supersedes the armed walk-up action, even one that starts
    // no movement of its own (a cast, an in-reach interaction). A click that
    // hit nothing at all shouldn't cancel the walk the player is already on.
    if (event.type === 'canvas_intent' && event.intent.type !== 'none') {
      const m = movingState()
      if (m) m.approach = null
    }
    dispatchQueuedPlayerControlEvent(event, createPlayerControlEventActions())
  }

  const playerControlMachine = createLocalPlayerControlMachine({
    dispatchEvent: dispatchPlayerControlEvent,
    stateActions: {
      onInteractionFinished,
      onPickupGrab,
      clearJumpFeedbackTimer,
      onInteractionRejected: () => {
        if (playerState.state === 'interact') exitObjectInteraction(false)
      },
      handleInteractKey: checkInteraction,
      handleKeyboard: updateKeyboardMovement,
      tick: updatePlayerMovement,
    },
  })

  export function updatePlayerControl(
    deltaTime: number,
    options: PlayerControlUpdateOptions
  ) {
    playerControlMachine.update(deltaTime, options)
  }

  // Hover overlays: signpost speech bubble, ground-item, prop and monster names.
  // Driven by pointermove (event-based, not per-frame) and raycast only against
  // those groups, throttled to ~20 Hz — negligible cost.
  let lastHoverRaycast = 0
  let lastHoverKey: string | null = null
  let hoverTrailing: ReturnType<typeof setTimeout> | null = null
  let pendingHoverEvent: MouseEvent | null = null

  const isMonsterDead = (id: string) =>
    monsterManager.monsters.get(id)?.state === 'dead'

  // An item being picked up is hidden through its parent's `visible`, which
  // raycasts ignore — so a hit on one that is gone or already in hand doesn't
  // count. A corpse likewise names nothing; both let the ray look past them.
  function isHoverable(target: HoverTarget): boolean {
    if (target.kind === 'groundItem') {
      const item = groundItemManager.items.get(target.instanceId)
      return !!item && !item.inHand
    }
    if (target.kind === 'monster') return !isMonsterDead(target.monsterId)
    if (target.kind === 'player') {
      const player = get(gameStore).otherPlayers.get(target.playerId)
      return !!player && player.health > 0
    }
    return true
  }

  /** Display name for a player id (stall owner labels), self included. */
  function ownerName(playerId: number): string | null {
    const state = get(gameStore)
    if (state.currentPlayer?.id === playerId) {
      return state.currentPlayer.name ?? null
    }
    return state.otherPlayers.get(playerId)?.name ?? null
  }

  function runHover(event: MouseEvent) {
    lastHoverRaycast = performance.now()
    const target = inputHandler.processHover(event, {
      camera,
      objectMeshes,
      tipHatMeshes,
      stallMeshes,
      propMeshes,
      groundItemMeshes,
      monsterMeshes: monsterHoverMeshes,
      playerMeshes: playerHoverMeshes,
      isHoverable,
      ownerName,
    })
    const key = hoverTargetKey(target)
    if (key === lastHoverKey) return
    lastHoverKey = key
    hoverTarget.set(target)
  }

  function handlePointerHover(event: MouseEvent) {
    pendingHoverEvent = event
    const dt = performance.now() - lastHoverRaycast
    if (dt >= 50) {
      if (hoverTrailing) {
        clearTimeout(hoverTrailing)
        hoverTrailing = null
      }
      runHover(event)
    } else if (!hoverTrailing) {
      // Trailing edge: process the final position after the throttle window so a
      // quick flick off a signpost (then stop, without leaving the canvas)
      // doesn't strand the bubble over empty ground.
      hoverTrailing = setTimeout(() => {
        hoverTrailing = null
        if (pendingHoverEvent) runHover(pendingHoverEvent)
      }, 50 - dt)
    }
  }

  function clearHover() {
    if (hoverTrailing) {
      clearTimeout(hoverTrailing)
      hoverTrailing = null
    }
    if (lastHoverKey === null) return
    lastHoverKey = null
    hoverTarget.set(null)
  }

  // Crossing a dungeon boundary swaps the visible entity layers wholesale;
  // a resting cursor would otherwise keep a stale snapshot hover. Subscribed
  // below the hover state it clears: subscribe fires its callback right here,
  // and the `let`s above are TDZ until their declarations run.
  currentDungeonDepth.subscribe(() => clearHover())

  onMount(() => {
    preloadSwordHitSound()
    preloadSwordMissSound()
    preloadMonsterDeathSounds()
    preloadPlayerHurtSounds()
    preloadPlayerDeathSounds()
    preloadPropSounds()
    preloadFishingSounds()

    const removeInputListeners = inputHandler.setupEventListeners(
      renderer.domElement,
      handleCanvasClickIntent
    )

    const canvas = renderer.domElement
    // OrbitControls listens on the canvas's wrapper and captures the pointer
    // there on every mousedown; that retargets pointer events and fires a
    // spurious pointerleave on the canvas even though the cursor never moved.
    // A leave "to" an ancestor of the canvas can only be that retargeting —
    // a real leave lands on a sibling overlay, another element, or null.
    const handlePointerLeave = (e: PointerEvent) => {
      if (e.relatedTarget instanceof Node && e.relatedTarget.contains(canvas))
        return
      clearHover()
    }
    canvas.addEventListener('pointermove', handlePointerHover)
    canvas.addEventListener('pointerleave', handlePointerLeave)

    const unsubscribeNetworkEvents = subscribePlayerNetworkEvents({
      isCurrentPlayerEligibleForRespawn: () =>
        !!currentPlayer && currentPlayer.health <= 0,
      isCurrentPlayer: (id) => !!currentPlayer && currentPlayer.id === id,
      isInteracting: () => playerState.state === 'interact',
      onRespawned: transitionToRespawned,
      onPositionCorrected: applyPositionCorrection,
      onInteractionRejected: () =>
        enqueuePlayerControlEvent({ type: 'network_interaction_rejected' }),
    })

    return () => {
      removeInputListeners()
      canvas.removeEventListener('pointermove', handlePointerHover)
      canvas.removeEventListener('pointerleave', handlePointerLeave)
      clearHover()
      unsubscribeNetworkEvents()
      playerControlMachine.dispose()
      clearStandUpTimer()
      clearJumpFeedbackTimer()
      clearPropSwingTimers()
      // The store outlives this component (character select, logout).
      lastEmoteSync = null
      localEmoteAnim.set(null)
    }
  })
</script>

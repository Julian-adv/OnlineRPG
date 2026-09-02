<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { get } from 'svelte/store'
  import { SvelteMap } from 'svelte/reactivity'
  import PlayerModel from '../PlayerModel.svelte'
  import PlayerControl from '../PlayerControl.svelte'
  import type { PlayerControlEvent } from '../player-control/events'
  import type {
    ChatBubble,
    LocalPlayer,
    RemotePlayer,
  } from '../../stores/gameStore'
  import type { PlayerState } from '../../utils/movementUtils'
  import type Monster from '../Monster.svelte'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'
  import { remotePlayerManager } from '../../managers/remotePlayerManager'

  import {
    applyTorchFlickerWorld,
    TORCH_BASE_DISTANCE,
    TORCH_BASE_DECAY,
    TORCH_BASE_POSITION,
    TORCH_SHADOW_FAR,
    TORCH_SHADOW_MAP_SIZE,
    TORCH_SHADOW_BIAS,
  } from '../../utils/torchFlicker'
  import {
    playerVisualFloorLevel,
    playerInsideHouseId,
  } from '../../stores/housingStore'
  import { currentDungeonDepth } from '../../stores/dungeonStore'
  import { myFishing } from '../../stores/fishingStore'
  import { FishingAnimationName } from '../../types/animations'
  import { housingManager } from '../../managers/housingManager'
  import { campfireManager } from '../../managers/campfireManager'
  import {
    shortestWrappedDeltaX,
    unwrapWorldXNear,
  } from '../../terrain/world-wrap'
  import { OFFSCREEN_Y } from '../../utils/house-geo-utils'
  import { torchLightEnabled } from '../../stores/debugStore'
  import { localTorchEquipped } from '../../stores/inventoryStore'

  const TORCH_OFFSET = new THREE.Vector3(
    TORCH_BASE_POSITION.x,
    TORCH_BASE_POSITION.y,
    TORCH_BASE_POSITION.z
  )
  const Y_AXIS = new THREE.Vector3(0, 1, 0)

  interface Props {
    camera: THREE.OrthographicCamera | undefined
    cameraInitialized: boolean
    currentPlayer: LocalPlayer | null
    otherPlayers: Map<number, RemotePlayer>
    remotePlayers: Map<number, PlayerState>
    chatBubbles: Map<number, ChatBubble>
    currentPlayerState: PlayerState
    terrainMeshes: (THREE.Mesh | undefined)[]
    housingGroup: THREE.Group | null
    dungeonGroup: THREE.Group | null
    doorMeshes: THREE.Object3D[]
    objectMeshes: THREE.Object3D[]
    propMeshes: THREE.Object3D[]
    groundItemMeshes: THREE.Object3D[]
    tipHatMeshes: THREE.Object3D[]
    stallMeshes: THREE.Object3D[]
    mealMeshes: THREE.Object3D[]
    monsterModels: (Monster | undefined)[]
    playerAttackDuration: number
    heightManager: TerrainHeightManager
    /** Baked water surface height at a world XZ (for fishing cast detection). */
    waterSurfaceAt?: (x: number, z: number) => number
    onStateChange: (newState: PlayerState) => void
    onPlayerControlEvent?: (event: PlayerControlEvent) => void
    onAttackDuration: (duration: number) => void
    onCurrentPlayerDyingFinished?: () => void
    isCurrentPlayerLoading?: boolean
    torchEffectsDisabled?: boolean
    playerControl?: PlayerControl
    currentPlayerModel?: PlayerModel | null
    otherPlayerModels?: (PlayerModel | undefined)[]
    torchLightCastsShadow?: boolean
    torchShadowMapSize?: number
    /** Per-frame provider of the current dungeon floor's wall-torch flame
     *  world-positions (empty when not underground). Pulled fresh each frame —
     *  the array is swapped on floor rebuild, so it must not be cached. */
    wallTorchPositions?: () => THREE.Vector3[]
    /** World positions of burning hearth flames (placed furniture). */
    hearthFirePositions?: () => THREE.Vector3[]
    /** Flame world-positions of wall torches placed in houses. */
    houseTorchPositions?: () => THREE.Vector3[]
  }

  let {
    camera,
    cameraInitialized,
    currentPlayer,
    otherPlayers,
    remotePlayers,
    chatBubbles,
    currentPlayerState,
    terrainMeshes,
    housingGroup,
    dungeonGroup,
    doorMeshes,
    objectMeshes,
    propMeshes,
    groundItemMeshes,
    tipHatMeshes,
    stallMeshes,
    mealMeshes,
    monsterModels,
    playerAttackDuration,
    heightManager,
    waterSurfaceAt,
    onStateChange,
    onPlayerControlEvent,
    onAttackDuration,
    onCurrentPlayerDyingFinished,
    isCurrentPlayerLoading = $bindable(false),
    torchEffectsDisabled = false,
    playerControl = $bindable<PlayerControl>(),
    currentPlayerModel = $bindable<PlayerModel | null>(null),
    otherPlayerModels = $bindable<(PlayerModel | undefined)[]>([]),
    torchLightCastsShadow = true,
    torchShadowMapSize = TORCH_SHADOW_MAP_SIZE,
    wallTorchPositions,
    hearthFirePositions,
    houseTorchPositions,
  }: Props = $props()

  // Sync attack animation duration to remote player manager
  $effect(() => {
    remotePlayerManager.attackAnimationDuration = playerAttackDuration
  })

  // Local-player fishing stance: overrides only the 'idle' state, so
  // movement/attack win and the PlayerControl FSM stays untouched.
  let fishingCastDone = $state(false)
  $effect(() => {
    if ($myFishing.phase === 'casting') fishingCastDone = false
  })
  const fishingOverrideActive = $derived(
    $myFishing.phase !== 'idle' && currentPlayerState.state === 'idle'
  )
  const effectivePlayerState = $derived(
    fishingOverrideActive ? 'interact' : currentPlayerState.state
  )
  const effectiveInteractionAnim = $derived(
    fishingOverrideActive
      ? $myFishing.phase === 'casting' && !fishingCastDone
        ? FishingAnimationName.CAST
        : FishingAnimationName.IDLE
      : currentPlayerState.interactionAnim
  )
  const effectiveInteractionCounter = $derived(
    fishingOverrideActive ? undefined : currentPlayerState.interactionCounter
  )

  // Visual floor: matches what remotes report, so a player on the stairs isn't
  // hidden from the floor they're still on. See playerVisualFloorLevel.
  let localFloorLevel = $derived($playerVisualFloorLevel)
  let localHouseId = $derived($playerInsideHouseId)
  let localDungeonDepth = $derived($currentDungeonDepth)
  let isUnderground = $derived(localDungeonDepth >= 1)

  function isRemotePlayerVisible(
    remoteFloorLevel: number,
    pos: { x: number; y: number; z: number }
  ): boolean {
    // Dungeon: only players on the same depth are visible; from the
    // surface, underground players are hidden (and vice versa).
    if (localDungeonDepth >= 1) {
      return remoteFloorLevel === -localDungeonDepth
    }
    if (remoteFloorLevel < 0) return false
    const remoteHouse = housingManager.findHouseAtPoint(pos.x, pos.y, pos.z)
    if (localHouseId) {
      return (
        remoteFloorLevel === localFloorLevel && remoteHouse?.id === localHouseId
      )
    }
    return remoteHouse == null
  }

  let remoteVisibility = $derived.by(() => {
    const map = new SvelteMap<number, boolean>()
    for (const [id, player] of otherPlayers) {
      const rp = remotePlayers.get(id)
      map.set(
        id,
        rp ? isRemotePlayerVisible(player.floorLevel, rp.position) : false
      )
    }
    return map
  })

  // Unified torch: exactly one PointLight for the entire scene.
  // Priority: local player's torch (if ON) > closest visible remote player
  // with torchOn. When no candidate, intensity drops to 0. Keeping the
  // PointLight count at a constant 1 avoids WebGPU pipeline recompile stalls.
  //
  // Position/intensity are driven imperatively from the game loop (not a
  // $derived) because currentPlayer.position is a mutated plain object that
  // Svelte reactivity cannot track. The game loop runs every frame anyway,
  // so recomputing the target here has no extra cost.
  let unifiedTorchLight = $state<THREE.PointLight | undefined>(undefined)

  // mapSize is only read when the cube map is allocated, so a quality switch
  // mid-session needs the old map dropped for the new size to take effect.
  $effect(() => {
    const map = unifiedTorchLight?.shadow?.map
    if (map && map.width !== torchShadowMapSize) {
      map.dispose()
      unifiedTorchLight!.shadow.map = null
    }
  })

  let unifiedTorchFlickerTime = 0
  const _unifiedTorchTmp = new THREE.Vector3()
  const _torchOffsetTmp = new THREE.Vector3()

  // Wall-torch light pool: N shadowless PointLights parked on the nearest wall
  // torches (dungeon floors, house interiors). Always mounted so the scene's
  // PointLight count never changes — mounting on house entry caused a visible
  // pipeline-recompile stall. Unused slots idle at intensity 0. Shadows stay on
  // the single unified light, which skips the torch it already occupies.
  const WALL_TORCH_POOL_SIZE = 6
  /** Wall torches glow a touch dimmer than a held/player torch. */
  const WALL_TORCH_INTENSITY_SCALE = 0.65
  /** Beyond this (world metres) a pooled wall torch is left dark — keeps the
   *  shadowless glow from bleeding through walls into far rooms. */
  const WALL_TORCH_POOL_RANGE = 14
  const WALL_TORCH_POOL_RANGE_SQ = WALL_TORCH_POOL_RANGE * WALL_TORCH_POOL_RANGE
  const wallTorchSlots = Array.from({ length: WALL_TORCH_POOL_SIZE })
  let wallTorchLights = $state<(THREE.PointLight | undefined)[]>([])
  const wallTorchFlickerTimes = wallTorchSlots.map((_, i) => i * 0.7)
  /** Scratch reused each frame to rank wall torches by distance to the player. */
  const _wallTorchRanking: { idx: number; dist: number }[] = []
  let wallTorchPoolIdle = false

  // The bearer's y already resolves house/dungeon floors, so don't resample terrain
  function setTorchTargetFromPose(
    x: number,
    z: number,
    y: number,
    rotation: number
  ): THREE.Vector3 {
    _torchOffsetTmp.copy(TORCH_OFFSET).applyAxisAngle(Y_AXIS, rotation)
    return _unifiedTorchTmp.set(
      x + _torchOffsetTmp.x,
      y + _torchOffsetTmp.y,
      z + _torchOffsetTmp.z
    )
  }

  /** Height above the fire bed where the campfire's light sits. */
  const CAMPFIRE_LIGHT_HEIGHT = 1.0
  /** A campfire further than this is left to the torches. */
  const CAMPFIRE_LIGHT_RANGE = 14
  const CAMPFIRE_LIGHT_RANGE_SQ = CAMPFIRE_LIGHT_RANGE * CAMPFIRE_LIGHT_RANGE
  /** A fire throws more light than a torch in hand. */
  const CAMPFIRE_INTENSITY_SCALE = 1.25
  /** A hearth's log bed sits inside the arch; its light hovers a little above
   *  and in front of the bed so the shadow light isn't buried in stone. */
  const HEARTH_LIGHT_HEIGHT = 0.4
  const _fireLightTmp = new THREE.Vector3()

  /** Light position of the closest burning fire (campfire or hearth) within
   *  range. Surface only — the campfire layer hides underground. */
  function nearestFireLight(playerPos: {
    x: number
    z: number
  }): THREE.Vector3 | null {
    if (isUnderground) return null
    let bestDist = CAMPFIRE_LIGHT_RANGE_SQ
    let found = false
    for (const fire of campfireManager.fires.values()) {
      const dx = shortestWrappedDeltaX(playerPos.x, fire.x)
      const dz = fire.z - playerPos.z
      const dist = dx * dx + dz * dz
      if (dist < bestDist) {
        bestDist = dist
        found = true
        _fireLightTmp.set(fire.x, fire.y + CAMPFIRE_LIGHT_HEIGHT, fire.z)
      }
    }
    for (const fire of hearthFirePositions?.() ?? []) {
      const dx = shortestWrappedDeltaX(playerPos.x, fire.x)
      const dz = fire.z - playerPos.z
      const dist = dx * dx + dz * dz
      if (dist < bestDist) {
        bestDist = dist
        found = true
        _fireLightTmp.set(fire.x, fire.y + HEARTH_LIGHT_HEIGHT, fire.z)
      }
    }
    return found ? _fireLightTmp : null
  }

  /** Pick the unified shadow light's target. Returns the world position, the
   *  intensity it should burn at, and — when it landed on a wall torch — that
   *  torch's index (so the pool can skip it). */
  function computeUnifiedTorchTarget(
    wallPositions: THREE.Vector3[]
  ): { target: THREE.Vector3; wallIdx: number; scale: number } | null {
    if (torchEffectsDisabled) return null
    if (!currentPlayer) return null
    // A campfire outshines any torch, held or not: the one light goes to the
    // fire while the player is near it.
    const fire = nearestFireLight(currentPlayer.position)
    if (fire) {
      return {
        target: _unifiedTorchTmp.set(
          unwrapWorldXNear(currentPlayer.position.x, fire.x),
          fire.y,
          fire.z
        ),
        wallIdx: -1,
        scale: CAMPFIRE_INTENSITY_SCALE,
      }
    }
    if (get(localTorchEquipped) || get(torchLightEnabled)) {
      const p = currentPlayer.position
      return {
        target: setTorchTargetFromPose(p.x, p.z, p.y, currentPlayer.rotation),
        wallIdx: -1,
        scale: 1,
      }
    }
    // No lit player torch: the nearest lit source — remote torch or wall torch,
    // ranked together by distance — takes the shadow-casting light.
    const playerPos = currentPlayer.position
    let bestDist = Infinity
    let bestRp: PlayerState | null = null
    let bestWallIdx = -1
    for (const [id, player] of otherPlayers) {
      const rp = remotePlayers.get(id)
      if (!player.torchOn || !rp || !remoteVisibility.get(id)) continue
      const dx = shortestWrappedDeltaX(playerPos.x, rp.position.x)
      const dz = rp.position.z - playerPos.z
      const dist = dx * dx + dz * dz
      if (dist < bestDist) {
        bestDist = dist
        bestRp = rp
        bestWallIdx = -1
      }
    }
    for (let i = 0; i < wallPositions.length; i++) {
      const w = wallPositions[i]
      const dx = w.x - playerPos.x
      const dz = w.z - playerPos.z
      const dist = dx * dx + dz * dz
      if (dist < bestDist) {
        bestDist = dist
        bestRp = null
        bestWallIdx = i
      }
    }
    if (bestWallIdx >= 0) {
      return {
        target: _unifiedTorchTmp.copy(wallPositions[bestWallIdx]),
        wallIdx: bestWallIdx,
        scale: WALL_TORCH_INTENSITY_SCALE,
      }
    }
    if (bestRp) {
      const displayX = unwrapWorldXNear(playerPos.x, bestRp.position.x)
      return {
        target: setTorchTargetFromPose(
          displayX,
          bestRp.position.z,
          bestRp.position.y,
          bestRp.rotation
        ),
        wallIdx: -1,
        scale: 1,
      }
    }
    return null
  }

  /** Park the pool's lights on the nearest wall torches (skipping `occupiedIdx`,
   *  already lit by the unified shadow light), idling the leftover slots. */
  function updateWallTorchPool(
    deltaTime: number,
    wallPositions: THREE.Vector3[],
    occupiedIdx: number
  ) {
    if (wallTorchLights.length === 0) return
    if (wallPositions.length === 0 && wallTorchPoolIdle) return
    wallTorchPoolIdle = wallPositions.length === 0
    const playerPos = currentPlayer?.position
    _wallTorchRanking.length = 0
    if (playerPos) {
      for (let i = 0; i < wallPositions.length; i++) {
        if (i === occupiedIdx) continue
        const w = wallPositions[i]
        const dx = w.x - playerPos.x
        const dz = w.z - playerPos.z
        const dist = dx * dx + dz * dz
        if (dist <= WALL_TORCH_POOL_RANGE_SQ)
          _wallTorchRanking.push({ idx: i, dist })
      }
      _wallTorchRanking.sort((a, b) => a.dist - b.dist)
    }
    for (let slot = 0; slot < wallTorchLights.length; slot++) {
      const light = wallTorchLights[slot]
      if (!light) continue
      const ranked = _wallTorchRanking[slot]
      if (ranked) {
        const w = wallPositions[ranked.idx]
        wallTorchFlickerTimes[slot] = applyTorchFlickerWorld(
          light,
          wallTorchFlickerTimes[slot],
          deltaTime,
          w.x,
          w.y,
          w.z,
          WALL_TORCH_INTENSITY_SCALE
        )
      } else {
        light.intensity = 0
      }
    }
  }

  export function updateUnifiedTorchFlicker(deltaTime: number) {
    const torchSource = isUnderground
      ? wallTorchPositions
      : localHouseId != null
        ? houseTorchPositions
        : undefined
    const wallPositions = torchEffectsDisabled ? [] : (torchSource?.() ?? [])
    let occupiedWallIdx = -1
    if (unifiedTorchLight) {
      const result = computeUnifiedTorchTarget(wallPositions)
      if (result) {
        occupiedWallIdx = result.wallIdx
        unifiedTorchFlickerTime = applyTorchFlickerWorld(
          unifiedTorchLight,
          unifiedTorchFlickerTime,
          deltaTime,
          result.target.x,
          result.target.y,
          result.target.z,
          result.scale
        )
      } else {
        unifiedTorchLight.intensity = 0
      }
    }
    updateWallTorchPool(deltaTime, wallPositions, occupiedWallIdx)
  }

  export function getUnifiedTorchLight(): THREE.PointLight | undefined {
    return unifiedTorchLight
  }
</script>

{#if camera && currentPlayer}
  <PlayerControl
    bind:this={playerControl}
    {waterSurfaceAt}
    {onStateChange}
    {camera}
    {heightManager}
    groundMeshes={localDungeonDepth >= 1 && dungeonGroup
      ? [dungeonGroup]
      : [
          ...(terrainMeshes.filter(
            (mesh) => mesh !== undefined
          ) as THREE.Mesh[]),
          ...(housingGroup ? [housingGroup] : []),
        ]}
    monsterMeshes={monsterModels
      .map((model) => model?.getMeshGroup())
      .filter((group) => group !== undefined) as THREE.Group[]}
    monsterHoverMeshes={monsterModels
      .map((model) => model?.getHoverMeshGroup())
      .filter((group) => group !== undefined) as THREE.Group[]}
    npcMeshes={(otherPlayerModels ?? [])
      .map((model) => model?.getModelGroup())
      .filter(
        (group): group is THREE.Group =>
          group !== undefined && group.userData.npcPlayerId != null
      )}
    playerMeshes={(otherPlayerModels ?? [])
      .map((model) => model?.getModelGroup())
      .filter((group): group is THREE.Group => group !== undefined)}
    playerHoverMeshes={(otherPlayerModels ?? [])
      .map((model) => model?.getHoverMeshGroup())
      .filter((group): group is THREE.Group => group !== undefined)}
    {doorMeshes}
    {objectMeshes}
    {propMeshes}
    {groundItemMeshes}
    {tipHatMeshes}
    {stallMeshes}
    {mealMeshes}
    attackCooldown={playerAttackDuration}
  />
{/if}

{#if currentPlayer && cameraInitialized && camera}
  <PlayerModel
    bind:this={currentPlayerModel}
    position={currentPlayer.position}
    name={currentPlayer.name}
    title={currentPlayer.title}
    isCurrentPlayer={true}
    playerState={effectivePlayerState}
    interactionAnim={effectiveInteractionAnim}
    interactionCounter={effectiveInteractionCounter}
    interactOffsetY={currentPlayerState.interactOffsetY}
    attackCounter={currentPlayerState.attackCounter}
    hitCounter={currentPlayer.hitCounter}
    speed={currentPlayerState.speed}
    rotation={currentPlayerState.rotation}
    movementMode={currentPlayerState.movementMode}
    {camera}
    chatBubble={chatBubbles.get(currentPlayer.id)?.message}
    chatBubbleAt={chatBubbles.get(currentPlayer.id)?.timestamp}
    characterClass={currentPlayer.characterClass}
    gender={currentPlayer.gender}
    health={currentPlayer.health}
    maxHealth={currentPlayer.maxHealth}
    {onAttackDuration}
    onDyingFinished={onCurrentPlayerDyingFinished}
    onInteractionFinished={() => {
      // The finished cast hands over to the looping fishing idle; the event
      // still goes to the FSM, which ignores anims it didn't start.
      if (fishingOverrideActive) fishingCastDone = true
      onPlayerControlEvent?.({ type: 'anim_interaction_finished' })
    }}
    onPickupGrab={() => {
      onPlayerControlEvent?.({ type: 'anim_pickup_grab' })
    }}
    bind:isLoading={isCurrentPlayerLoading}
    lastDamageInfo={currentPlayer.lastDamageInfo}
    lastRegenInfo={currentPlayer.lastRegenInfo}
    lastGoldInfo={currentPlayer.lastGoldInfo}
    {torchEffectsDisabled}
  />
{/if}

{#if cameraInitialized && camera}
  {#each [...otherPlayers.values()] as player, index (player.id)}
    {@const remotePlayer = remotePlayers.get(player.id)}
    {#if remotePlayer}
      {@const visible = remoteVisibility.get(player.id) ?? false}
      {@const displayX = currentPlayer
        ? unwrapWorldXNear(currentPlayer.position.x, remotePlayer.position.x)
        : remotePlayer.position.x}
      <!-- position.y is ground-resampled per tick by remotePlayerManager -->
      {@const baseY = remotePlayer.position.y}
      <PlayerModel
        bind:this={otherPlayerModels[index]}
        position={new THREE.Vector3(
          displayX,
          visible ? baseY : OFFSCREEN_Y,
          remotePlayer.position.z
        )}
        name={player.name}
        title={player.title}
        isCurrentPlayer={false}
        playerState={remotePlayer.state}
        interactionAnim={remotePlayer.interactionAnim}
        interactionCounter={remotePlayer.interactionCounter}
        interactOffsetY={remotePlayer.interactOffsetY}
        attackCounter={remotePlayer.attackCounter}
        hitCounter={remotePlayerManager.hitCounters.get(player.id)}
        speed={remotePlayer.speed}
        rotation={remotePlayer.rotation}
        movementMode={remotePlayer.movementMode}
        {camera}
        chatBubble={chatBubbles.get(player.id)?.message}
        chatBubbleAt={chatBubbles.get(player.id)?.timestamp}
        characterClass={player.characterClass}
        gender={player.gender}
        health={player.health}
        maxHealth={player.maxHealth}
        torchOn={player.torchOn}
        mainHand={player.mainHand}
        back={player.back}
        backColor={player.backColor}
        backTexture={player.backTexture}
        {torchEffectsDisabled}
        npcPlayerId={player.isOfficialNpc ? player.id : undefined}
        remotePlayerId={player.id}
        floorLevel={player.floorLevel}
        {heightManager}
        onInteractionFinished={() =>
          remotePlayerManager.handleInteractionFinished(player.id)}
      />
    {/if}
  {/each}

  <!-- Unified point light. Mounted exactly once, priority:
       local torch > closest visible remote torch. Shadow mode is fixed by the
       effective graphics preset (mobile keeps the light but skips shadow maps).
       Position/intensity are driven from the game loop. -->
  {#if !torchEffectsDisabled}
    <T.PointLight
      bind:ref={unifiedTorchLight}
      position={[0, 0, 0]}
      color="#ffcc66"
      intensity={0}
      distance={TORCH_BASE_DISTANCE}
      decay={TORCH_BASE_DECAY}
      castShadow={torchLightCastsShadow}
      shadow.mapSize.width={torchShadowMapSize}
      shadow.mapSize.height={torchShadowMapSize}
      shadow.camera.near={1.5}
      shadow.camera.far={TORCH_SHADOW_FAR}
      shadow.bias={TORCH_SHADOW_BIAS}
      shadow.normalBias={0.005}
      shadow.radius={2}
    />

    <!-- Wall-torch glow pool: a fixed N of shadowless point lights, parked on the
         nearest wall torches each frame (see updateWallTorchPool). Always
         mounted so the light count never churns. -->
    {#each wallTorchSlots as _slot, i (i)}
      <T.PointLight
        bind:ref={wallTorchLights[i]}
        position={[0, 0, 0]}
        color="#ffcc66"
        intensity={0}
        distance={TORCH_BASE_DISTANCE}
        decay={TORCH_BASE_DECAY}
        castShadow={false}
      />
    {/each}
  {/if}
{/if}

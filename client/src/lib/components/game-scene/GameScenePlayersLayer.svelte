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
  import { dungeonManager } from '../../managers/dungeonManager'
  import { housingManager } from '../../managers/housingManager'
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
  }: Props = $props()

  // Sync attack animation duration to remote player manager
  $effect(() => {
    remotePlayerManager.attackAnimationDuration = playerAttackDuration
  })

  // Visual floor: matches what remotes report, so a player on the stairs isn't
  // hidden from the floor they're still on. See playerVisualFloorLevel.
  let localFloorLevel = $derived(Math.max(0, $playerVisualFloorLevel))
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

  // Wall-torch light pool: a *fixed* set of non-shadow PointLights, each parked
  // on one of the N nearest dungeon wall torches so the floor glows even with no
  // lit player torch around. Mounted only while underground (below), so the
  // scene's PointLight count steps 1->1+N on dungeon entry and back on exit.
  // That deviates from the unified light's "always-mounted, constant 1" rule on
  // purpose: the unified light stays constant because remote torch-bearers come
  // and go *mid-play* (a recompile stall there is visible), whereas this pool's
  // count only changes on a dungeon enter/exit — a hard scene transition that
  // already loads/unloads all the floor geometry, so its one-time (then cached)
  // pipeline recompile is masked. Always-mounting these N everywhere instead
  // would tax the whole overworld with N dead point lights it never needs.
  // Within the dungeon the count is fixed (unused slots idle at intensity 0), so
  // moving between rooms/floors never churns the pipeline. Shadow casting stays
  // on the *single* unified light (below): when no player torch is lit it
  // relocates to the nearest wall torch and casts its shadow there, matching how
  // a remote player's torch is treated; that torch is then skipped here so it
  // isn't double-lit.
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

  function setTorchTargetFromPose(
    x: number,
    z: number,
    fallbackY: number,
    rotation: number
  ): THREE.Vector3 {
    const y =
      dungeonManager.sampleHeightAt(x, z) ??
      heightManager.getHeightAtWorldPosition(x, z) ??
      fallbackY
    _torchOffsetTmp.copy(TORCH_OFFSET).applyAxisAngle(Y_AXIS, rotation)
    return _unifiedTorchTmp.set(
      x + _torchOffsetTmp.x,
      y + _torchOffsetTmp.y,
      z + _torchOffsetTmp.z
    )
  }

  /** Pick the unified shadow light's target. Returns the world position plus, when
   *  it landed on a wall torch, that torch's index (so the pool can skip it). */
  function computeUnifiedTorchTarget(
    wallPositions: THREE.Vector3[]
  ): { target: THREE.Vector3; wallIdx: number } | null {
    if (torchEffectsDisabled) return null
    if (!currentPlayer) return null
    if (get(localTorchEquipped) || get(torchLightEnabled)) {
      const p = currentPlayer.position
      return {
        target: setTorchTargetFromPose(p.x, p.z, p.y, currentPlayer.rotation),
        wallIdx: -1,
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
    const wallPositions =
      isUnderground && !torchEffectsDisabled
        ? (wallTorchPositions?.() ?? [])
        : []
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
          occupiedWallIdx >= 0 ? WALL_TORCH_INTENSITY_SCALE : 1
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
    npcMeshes={(otherPlayerModels ?? [])
      .map((model) => model?.getModelGroup())
      .filter(
        (group): group is THREE.Group =>
          group !== undefined && group.userData.npcPlayerId != null
      )}
    {doorMeshes}
    {objectMeshes}
    {propMeshes}
    {groundItemMeshes}
    attackCooldown={playerAttackDuration}
  />
{/if}

{#if currentPlayer && cameraInitialized && camera}
  <PlayerModel
    bind:this={currentPlayerModel}
    position={currentPlayer.position}
    name={currentPlayer.name}
    isCurrentPlayer={true}
    playerState={currentPlayerState.state}
    interactionAnim={currentPlayerState.interactionAnim}
    interactOffsetY={currentPlayerState.interactOffsetY}
    attackCounter={currentPlayerState.attackCounter}
    speed={currentPlayerState.speed}
    rotation={currentPlayerState.rotation}
    movementMode={currentPlayerState.movementMode}
    {camera}
    chatBubble={chatBubbles.get(currentPlayer.id)?.message}
    characterClass={currentPlayer.characterClass}
    gender={currentPlayer.gender}
    health={currentPlayer.health}
    maxHealth={currentPlayer.maxHealth}
    {onAttackDuration}
    onDyingFinished={onCurrentPlayerDyingFinished}
    onInteractionFinished={() => {
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
        isCurrentPlayer={false}
        playerState={remotePlayer.state}
        interactionAnim={remotePlayer.interactionAnim}
        interactOffsetY={remotePlayer.interactOffsetY}
        attackCounter={remotePlayer.attackCounter}
        speed={remotePlayer.speed}
        rotation={remotePlayer.rotation}
        movementMode={remotePlayer.movementMode}
        {camera}
        chatBubble={chatBubbles.get(player.id)?.message}
        characterClass={player.characterClass}
        gender={player.gender}
        health={player.health}
        maxHealth={player.maxHealth}
        torchOn={player.torchOn}
        {torchEffectsDisabled}
        npcPlayerId={player.isOfficialNpc ? player.id : undefined}
        onInteractionFinished={() =>
          remotePlayerManager.handleStopInteraction(player.id)}
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
         nearest wall torches each frame (see updateWallTorchPool). Mounted only
         underground; the slot count is constant so the light count never churns
         mid-floor. -->
    {#if isUnderground}
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
{/if}

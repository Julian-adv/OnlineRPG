<script lang="ts">
  import { T, useTask, useThrelte } from '@threlte/core'
  import { OrbitControls } from '@threlte/extras'
  import * as THREE from 'three'
  import { ClippingGroup, type WebGPURenderer } from 'three/webgpu'
  import { onMount } from 'svelte'
  import {
    gameStore,
    resetGameStore,
    hoveredSignpost,
    type LocalPlayer,
    type RemotePlayer,
    type ChatBubble,
  } from '../stores/gameStore'
  import SignpostBubble from './ChatBubble.svelte'
  import {
    startChatBubbleChecker,
    stopChatBubbleChecker,
  } from '../managers/chatBubbleManager'
  import { remotePlayerManager } from '../managers/remotePlayerManager'
  import { networkManager } from '../network/socket'
  import { monsterManager } from '../managers/monsterManager'
  import type PlayerModel from './PlayerModel.svelte'
  import type PlayerControl from './PlayerControl.svelte'
  import type { PlayerControlEvent } from './player-control/events'
  import type Monster from './Monster.svelte'
  import GameSceneTerrainLayer from './game-scene/GameSceneTerrainLayer.svelte'
  import GameSceneWaterFieldLayer from './game-scene/GameSceneWaterFieldLayer.svelte'
  import GameSceneRiverRocksLayer from './game-scene/GameSceneRiverRocksLayer.svelte'
  import GameSceneShoreSprayLayer from './game-scene/GameSceneShoreSprayLayer.svelte'
  import GameSceneGrassLayer from './game-scene/GameSceneGrassLayer.svelte'
  import GameSceneTreeLayer from './game-scene/GameSceneTreeLayer.svelte'
  import GameSceneWindParticles from './game-scene/GameSceneWindParticles.svelte'
  import GameSceneHousingLayer from './game-scene/GameSceneHousingLayer.svelte'
  import GameSceneDungeonLayer from './game-scene/GameSceneDungeonLayer.svelte'
  import { isUnderground } from '../stores/dungeonStore'
  import {
    playerFloorLevel,
    playerFloorOffset,
    playerInsideHouseId,
  } from '../stores/housingStore'
  import { drainTileWork } from '../utils/tileWorkQueue'
  import { FRAME_TIME_MS, FRAME_TOLERANCE_MS } from '../utils/frameTiming'
  import { createRenderCadence } from '../utils/renderCadence'
  import { bootstrapSceneAssets } from './game-scene/asset-bootstrap'
  import GameScenePlayersLayer from './game-scene/GameScenePlayersLayer.svelte'
  import GameSceneMonstersLayer from './game-scene/GameSceneMonstersLayer.svelte'
  import GameSceneGroundItemsLayer from './game-scene/GameSceneGroundItemsLayer.svelte'
  import FishingBobber from './FishingBobber.svelte'
  import { fishingBobbers } from '../stores/fishingStore'
  import MapEditorCursor from './map-editor/MapEditorCursor.svelte'
  import ZoneOverlay from './map-editor/ZoneOverlay.svelte'
  import RoadOverlay from './map-editor/RoadOverlay.svelte'
  import NpcWaypointOverlay from './map-editor/NpcWaypointOverlay.svelte'
  import ObjectOverlay from './map-editor/ObjectOverlay.svelte'
  import HousingEditorCursor from './map-editor/HousingEditorCursor.svelte'
  import { type PlayerState } from '../utils/movementUtils'
  import {
    SUN_MAX_INTENSITY,
    computeSunLightSnapshot,
    type SunLightSnapshot,
    type CalendarDate,
  } from '../utils/celestialSimulation'
  import { cameraDistance, cameraResetNonce } from '../stores/cameraStore'
  import {
    timeScale,
    sunTimeScale,
    sunDebugOffset,
    serverGameTime,
  } from '../stores/timeStore'
  import {
    debugVisible,
    cameraRotationEnabled,
    playerDebugInfo,
    mapEditorMode,
    teleportLoading,
    debugSpeedMode,
    refractionEnabled,
    reflectionEnabled,
    housingEditorMode,
  } from '../stores/debugStore'
  import {
    editorPanOffset,
    editorHeightManager,
    editorSplatManager,
    editorGrassDataManager,
    editorTreeDataManager,
    editorZoneManager,
    terrainForceRebuild,
    currentObjectData,
  } from '../stores/editorStore'
  import { ZoneManager } from '../managers/zoneManager'
  import { initFpsCounting, tickFps, tickRenderFps } from './FPSCounter.svelte'
  import { tickWavePhase } from './WavePhaseDebug.svelte'
  import {
    eclipseState,
    setGameDate,
    setGameHour,
  } from './GameTimeWidget.svelte'
  import {
    DEFAULT_CAMERA_OFFSET,
    INITIAL_DISTANCE,
    ORTHOGRAPHIC_DEFAULT_ZOOM,
    ORTHOGRAPHIC_FRUSTUM_HEIGHT,
    calculateCameraOffset as getCameraOffsetFromScene,
    resetCameraRotation as resetCameraRotationToDefault,
    resetCameraToInitialState as resetCameraToDefaultState,
    updateCameraWithOffset as applyCameraOffset,
    updateOrthographicFrustum,
  } from './game-scene/camera-utils'
  import { type TerrainTile } from './game-scene/terrain-utils'
  import { createLoopProfiler } from './game-scene/loop-profiler'
  import { createRenderProfiler } from './game-scene/render-profiler'
  import {
    setupCsmShadow,
    applyGraphicsPreset,
  } from './game-scene/renderer-quality'
  import {
    runRenderPasses,
    recordRenderProfilerStats,
  } from './game-scene/render-passes'
  import { createSceneLightingController } from './game-scene/scene-lighting'
  import { TerrainHeightManager } from '../managers/terrainHeightManager'
  import { TerrainSplatManager } from '../managers/terrainSplatManager'
  import { TerrainGrassDataManager } from '../managers/terrainGrassDataManager'
  import { TerrainTreeDataManager } from '../managers/terrainTreeDataManager'
  import { WaterFieldManager } from '../managers/waterFieldManager'
  import { createCalendarSystem } from './game-scene/calendar-system'
  import { createTerrainTileManager } from './game-scene/terrain-tile-manager'
  import { registerDebugConsole } from './game-scene/debug-console'
  import { initScene } from './game-scene/scene-init'
  import { createMultiPassRenderer } from './game-scene/multi-pass-rendering'
  import { OFFSCREEN_Y } from '../utils/house-geo-utils'
  import {
    graphicsQuality,
    getEffectivePreset,
  } from '../stores/graphicsSettings'

  interface Props {
    serverUrl: string
    onCurrentPlayerDyingFinished?: () => void
    isCurrentPlayerLoading?: boolean
    isSceneCompiling?: boolean
  }

  let {
    serverUrl,
    onCurrentPlayerDyingFinished,
    isCurrentPlayerLoading = $bindable(false),
    isSceneCompiling = $bindable(true),
  }: Props = $props()

  let currentPlayer = $state<LocalPlayer | null>(null)
  let otherPlayers = $state<Map<number, RemotePlayer>>(new Map())
  let chatBubbles = $state<Map<number, ChatBubble>>(new Map())
  let camera = $state<THREE.OrthographicCamera | undefined>(undefined)
  let directionalLight = $state<THREE.DirectionalLight | undefined>(undefined)
  let ambientLight = $state<THREE.AmbientLight | undefined>(undefined)
  let placeholderShadowLight = $state<THREE.PointLight | undefined>(undefined)
  let terrainMeshes = $state<(THREE.Mesh | undefined)[]>([])
  let terrainGroup = $state<THREE.Group | undefined>(undefined)
  let syncTileMeshes = $state<() => void>(() => {})
  let terrainGeometry = $state<THREE.BufferGeometry | null>(null)
  let terrainTiles = $state<TerrainTile[]>([])
  let terrainCenterChunk = $state({ x: 0, z: 0 })
  const terrainHeightManager = new TerrainHeightManager()
  const terrainSplatManager = new TerrainSplatManager()
  const terrainGrassDataManager = new TerrainGrassDataManager(
    terrainHeightManager
  )
  const terrainTreeDataManager = new TerrainTreeDataManager(
    terrainHeightManager
  )
  const waterFieldManager = new WaterFieldManager()
  monsterManager.heightManager = terrainHeightManager
  monsterManager.splatManager = terrainSplatManager
  remotePlayerManager.heightManager = terrainHeightManager
  editorHeightManager.set(terrainHeightManager)
  editorSplatManager.set(terrainSplatManager)
  editorZoneManager.set(new ZoneManager())
  editorGrassDataManager.set(terrainGrassDataManager)
  editorTreeDataManager.set(terrainTreeDataManager)
  let waterNormalMap = $state<THREE.Texture | null>(null)
  let waterFoamMap = $state<THREE.Texture | null>(null)
  let waterCausticsMap = $state<THREE.Texture | null>(null)
  let waterTime = $state(0)
  let waterSunDir = $state<THREE.Vector3 | null>(null)
  let waterSunColor = $state<THREE.Color | null>(null)
  let waterCamDir = $state<THREE.Vector3 | null>(null)
  let waterMoonBrightness = $state(0)
  let waterTorchLight = $state<THREE.PointLight | null>(null)
  let waterGroup = $state<THREE.Group | undefined>(undefined)
  let waterLayerRef = $state<GameSceneWaterFieldLayer | undefined>(undefined)
  let riverRocksRef = $state<GameSceneRiverRocksLayer | undefined>(undefined)
  let shoreSprayRef = $state<GameSceneShoreSprayLayer | undefined>(undefined)
  let grassLayerRef = $state<GameSceneGrassLayer | undefined>(undefined)
  let treeLayerRef = $state<GameSceneTreeLayer | undefined>(undefined)
  let windParticlesRef = $state<GameSceneWindParticles | undefined>(undefined)
  let housingLayerRef = $state<GameSceneHousingLayer | undefined>(undefined)
  let dungeonLayerRef = $state<GameSceneDungeonLayer | undefined>(undefined)
  let groundItemsLayerRef = $state<GameSceneGroundItemsLayer | undefined>(
    undefined
  )
  let objectOverlayRef = $state<ObjectOverlay | undefined>(undefined)
  let signpostBubbleRef = $state<SignpostBubble | undefined>(undefined)
  let signpostBubblePos = $derived(
    $hoveredSignpost
      ? new THREE.Vector3(
          $hoveredSignpost.x,
          $hoveredSignpost.y,
          $hoveredSignpost.z
        )
      : new THREE.Vector3()
  )
  let entityClipGroup = $state<ClippingGroup | undefined>(undefined)
  /** ClippingGroup instance with Y=0 clip plane, starts disabled. */
  const entityClipGroupObj = (() => {
    const g = new ClippingGroup()
    g.clippingPlanes = [new THREE.Plane(new THREE.Vector3(0, 1, 0), 0)]
    g.enabled = false
    return g
  })()
  const waterSunDirTmp = new THREE.Vector3()
  const waterCamDirTmp = new THREE.Vector3()
  let refractionManager = $state<
    import('../managers/refractionRenderManager').RefractionRenderManager | null
  >(null)
  let refractionTexture = $state<THREE.Texture | null>(null)
  let reflectionManager = $state<
    import('../managers/reflectionRenderManager').ReflectionRenderManager | null
  >(null)
  let reflectionTexture = $state<THREE.Texture | null>(null)
  let cameraInitialized = $state(false)
  let playerAttackDuration = $state(1.533) // Default from slash1 animation (data/animation_durations.json)

  const multiPassRenderer = createMultiPassRenderer()
  const graphicsPreset = $derived(getEffectivePreset($graphicsQuality))

  // Driven by the fixed simulation step below, so renders line up one-to-one
  // with the frames that produced them instead of running on a second clock.
  const renderCadence = $derived(
    createRenderCadence(graphicsPreset.maxRenderFps)
  )

  // Track whether all initial data is loaded (terrain + splat + grass assets).
  // The loading dialog stays until frames render smoothly (pipeline compilation
  // done by Threlte's render loop under the dialog overlay).
  let initialDataReady = false
  let smoothFrameCount = 0
  const SMOOTH_FRAME_THRESHOLD = 3 // consecutive smooth frames to consider ready
  const SMOOTH_FRAME_TIME_MS = 100 // frame must be under this to count

  // Camera follow system
  let cameraTarget = $state<[number, number, number]>([0, 0, 0])

  const {
    size,
    renderer: _renderer,
    scene,
    invalidate,
    renderStage,
  } = useThrelte()
  // Cast renderer — Threlte types it as WebGLRenderer but we use WebGPURenderer via createRenderer
  const renderer = _renderer as unknown as WebGPURenderer

  // Threlte skips the render stage entirely on frames it doesn't draw, so this
  // counts real renders rather than loop iterations.
  useTask(tickRenderFps, { stage: renderStage, autoInvalidate: false })
  let viewportSize = $state({ width: 1, height: 1 })

  const CAMERA_OFFSET = import.meta.hot?.data?.cameraOffset ?? {
    ...DEFAULT_CAMERA_OFFSET,
  }
  let _hmrCameraZoom: number | null = import.meta.hot?.data?.cameraZoom ?? null
  let _hmrCameraInitialized: boolean =
    import.meta.hot?.data?.cameraInitialized ?? false

  if (import.meta.hot) {
    import.meta.hot.dispose((data) => {
      data.cameraOffset = { ...CAMERA_OFFSET }
      data.cameraInitialized = cameraInitialized
      data.cameraZoom = camera?.zoom ?? null
    })
  }

  // Initialize camera as soon as both camera ref and currentPlayer are available
  $effect(() => {
    if (!cameraInitialized && camera && currentPlayer) {
      resetCameraToInitialState()
      cameraInitialized = true
    }
  })

  // Once pipeline compilation is done, freeze the placeholder point light's
  // shadow. It is positioned offscreen with intensity 0, so its shadow map
  // never needs to update — but three.js WebGPU still runs all 6 cube-face
  // shadow render passes every frame unless shadow.autoUpdate is disabled.
  $effect(() => {
    if (isSceneCompiling) return
    const light = placeholderShadowLight
    if (!light) return
    light.shadow.autoUpdate = false
    light.shadow.needsUpdate = false
  })

  // Reset camera rotation to default angle when debug mode is turned off or rotation is disabled
  let prevDebugVisible = $state(false)
  let prevRotationEnabled = $state(false)
  $effect(() => {
    const currentDebug = $debugVisible
    const currentRotation = $cameraRotationEnabled

    // Reset if debug mode was turned off OR rotation was just disabled
    if (
      (prevDebugVisible && !currentDebug) ||
      (prevRotationEnabled && !currentRotation)
    ) {
      resetCameraRotation()
    }

    prevDebugVisible = currentDebug
    prevRotationEnabled = currentRotation
  })

  function resetCameraRotation() {
    if (!currentPlayer || !camera) return
    cameraTarget = resetCameraRotationToDefault(camera, currentPlayer.position)
  }

  // Reset camera and pan offset when entering/leaving map editor mode
  let prevMapEditorMode = $state(false)
  $effect(() => {
    const current = $mapEditorMode
    if (prevMapEditorMode !== current) {
      // Clear pan offset first so resetCameraRotation computes the correct distance
      editorPanOffset.set({ x: 0, z: 0 })
      resetCameraRotation()
    }
    prevMapEditorMode = current
  })

  $effect(() => {
    updateOrthographicFrustum(camera, viewportSize)
  })

  const sceneLighting = createSceneLightingController()

  // Underground render mode: hide the overworld while in a dungeon.
  // Visibility toggle only — never unmount (avoids pipeline recompiles).
  $effect(() => {
    const underground = $isUnderground
    if (terrainGroup) terrainGroup.visible = !underground
    if (waterGroup) waterGroup.visible = !underground
    const grassGroup = grassLayerRef?.getGroup()
    if (grassGroup) grassGroup.visible = !underground
    const treeGroup = treeLayerRef?.getGroup()
    if (treeGroup) treeGroup.visible = !underground
    const housingGroup = housingLayerRef?.getGroup()
    if (housingGroup) housingGroup.visible = !underground
    const windGroup = windParticlesRef?.getGroup?.()
    if (windGroup) windGroup.visible = !underground
    const riverRocksGroup = riverRocksRef?.getGroup?.()
    if (riverRocksGroup) riverRocksGroup.visible = !underground
    const shoreSprayGroup = shoreSprayRef?.getGroup?.()
    if (shoreSprayGroup) shoreSprayGroup.visible = !underground
    if (underground) {
      // The housing layer's per-frame detection is skipped underground;
      // clear its state so stale floor offsets can't leak into physics.
      playerInsideHouseId.set(null)
      playerFloorLevel.set(-1)
      playerFloorOffset.set(0)
    }
  })

  $effect(() => {
    if (!directionalLight) return
    setupCsmShadow(directionalLight)
  })

  $effect(() => {
    applyGraphicsPreset(renderer, graphicsPreset, directionalLight)
  })

  const calendarSystem = createCalendarSystem({
    onDateChanged: setGameDate,
  })

  let latestServerGameTime:
    | import('../stores/timeStore').ServerGameTime
    | null = null
  let latestSunTimeScale = 1

  // Game loop
  let gameLoopId = $state<number | null>(null)
  let lastFrameTime = $state(0)
  const MAX_CATCH_UP_STEPS = 5

  let loopProfileEnabled = false
  const loopProfiler = createLoopProfiler(() => loopProfileEnabled)

  const renderProfiler = createRenderProfiler(() => loopProfileEnabled)

  // Player state from PlayerControl
  let currentPlayerState = $state<PlayerState>({
    state: 'idle',
    speed: 0,
    rotation: 0,
    position: { x: 0, y: 0, z: 0 },
  })

  // References to PlayerModel components
  let currentPlayerModel = $state<PlayerModel | null>(null)
  let otherPlayerModels = $state<(PlayerModel | undefined)[]>([])

  // Reference to PlayerControl and PlayersLayer components
  let playerControl = $state<PlayerControl>()
  let playersLayer = $state<GameScenePlayersLayer>()
  let pendingPlayerControlEvents: PlayerControlEvent[] = []

  function enqueuePlayerControlEvent(event: PlayerControlEvent) {
    pendingPlayerControlEvents.push(event)
  }

  // Handle player state changes from PlayerControl
  function handlePlayerStateChange(newState: PlayerState) {
    currentPlayerState = newState
  }

  const tileManager = createTerrainTileManager({
    getTiles: () => terrainTiles,
    setTiles: (tiles) => {
      terrainTiles = tiles
    },
    getCenterChunk: () => terrainCenterChunk,
    setCenterChunk: (chunk) => {
      terrainCenterChunk = chunk
    },
  })

  // Force terrain rebuild when requested (e.g. after region delete/generate)
  let lastRebuildVersion = 0
  $effect(() => {
    const v = $terrainForceRebuild
    if (v > lastRebuildVersion) {
      lastRebuildVersion = v
      tileManager.resetForForceRebuild()
      terrainTreeDataManager.invalidateAll()
    }
  })

  gameStore.subscribe((state) => {
    currentPlayer = state.currentPlayer
    otherPlayers = state.otherPlayers
    chatBubbles = state.chatBubbles
  })

  // Monster models reference
  let monsterModels = $state<(Monster | undefined)[]>([])

  // Main game loop with 60fps throttling
  function gameLoop(currentTime: number) {
    tickFps(currentTime)

    const rawDeltaTime = currentTime - lastFrameTime
    const shouldRunFrame = rawDeltaTime >= FRAME_TIME_MS - FRAME_TOLERANCE_MS

    // Throttle to 60fps
    if (shouldRunFrame) {
      const unclampedSteps = Math.max(
        1,
        Math.floor((rawDeltaTime + FRAME_TOLERANCE_MS) / FRAME_TIME_MS)
      )
      const stepCount = Math.min(unclampedSteps, MAX_CATCH_UP_STEPS)
      const fixedDeltaTime = FRAME_TIME_MS * stepCount

      loopProfiler.onFrame(fixedDeltaTime, FRAME_TIME_MS)

      const frameWorkStart = performance.now()

      const realDeltaSeconds = fixedDeltaTime / 1000

      // Apply time scale for slow motion debugging
      const deltaTime = fixedDeltaTime * $timeScale
      const sunDeltaSeconds = realDeltaSeconds * $sunTimeScale
      calendarSystem.advance(sunDeltaSeconds)
      const serverHour = calendarSystem.getGameHour()
      const displayHour = serverHour + $sunDebugOffset
      setGameHour(displayHour, serverHour)

      // Calculate camera offset before player movement
      const cameraOffsetStart = performance.now()
      let cameraOffset = calculateCameraOffset()
      loopProfiler.record('cameraOffset', performance.now() - cameraOffsetStart)

      // Movement runs in map/housing editor so right-click-to-move works.
      const playerControlStart = performance.now()
      if (playerControl) {
        const events = pendingPlayerControlEvents
        pendingPlayerControlEvents = []
        playerControl.updatePlayerControl(deltaTime, {
          editorMode: $mapEditorMode || $housingEditorMode,
          events,
        })
      }
      tileManager.updateFromPlayerPosition(currentPlayer?.position ?? null)
      if (
        terrainTiles.length < graphicsPreset.terrainQueueDrainTilesBeforeStagger
      ) {
        tileManager.drainQueue()
      }
      drainTileWork(graphicsPreset.terrainTileWorkPerFrame)
      syncTileMeshes()
      // Finalize teleport once full 3x3 heightmap grid is loaded.
      // Underground the dungeon owns Y — never snap to terrain height.
      if (
        $teleportLoading &&
        currentPlayer &&
        terrainHeightManager.hasHeightDataForGrid(
          currentPlayer.position.x,
          currentPlayer.position.z
        )
      ) {
        if (!$isUnderground) {
          currentPlayer.position.y =
            terrainHeightManager.getHeightAtWorldPosition(
              currentPlayer.position.x,
              currentPlayer.position.z
            )
        }
        teleportLoading.set(false)
        resetCameraToInitialState()
        cameraOffset = { ...CAMERA_OFFSET }
      }
      loopProfiler.record(
        'playerControl',
        performance.now() - playerControlStart
      )

      // Update remote player interpolation
      const remoteInterpolationStart = performance.now()
      remotePlayerManager.update(deltaTime)
      loopProfiler.record(
        'remoteInterpolation',
        performance.now() - remoteInterpolationStart
      )

      // Update player model animations
      const currentPlayerAnimationStart = performance.now()
      if (currentPlayerModel) {
        currentPlayerModel.update(deltaTime / 1000)
      }
      loopProfiler.record(
        'currentPlayerAnimation',
        performance.now() - currentPlayerAnimationStart
      )

      // Update other player model animations
      const otherPlayerAnimationStart = performance.now()
      for (const playerModel of otherPlayerModels) {
        if (playerModel) {
          playerModel.update(deltaTime / 1000)
        }
      }
      loopProfiler.record(
        'otherPlayerAnimation',
        performance.now() - otherPlayerAnimationStart
      )

      // Keep the signpost hover bubble facing the camera
      signpostBubbleRef?.update()

      // Update unified torch light flickering (single shadow-casting light)
      playersLayer?.updateUnifiedTorchFlicker(deltaTime / 1000)
      // Capture torch ref once for the river shader. The PointLight mutates
      // its own position/intensity in place each frame, so a single stable
      // reference is enough — the shader reads .position / .intensity at
      // uniform-write time.
      if (!waterTorchLight) {
        const tl = playersLayer?.getUnifiedTorchLight()
        if (tl) waterTorchLight = tl
      }

      // Update monster animations
      const monsterAnimationStart = performance.now()
      for (const monsterModel of monsterModels) {
        if (monsterModel) {
          monsterModel.update(deltaTime / 1000, camera) // Convert ms to seconds for THREE.AnimationMixer
        }
      }
      loopProfiler.record(
        'monsterAnimation',
        performance.now() - monsterAnimationStart
      )

      // Update monster spawning logic
      const monsterLogicStart = performance.now()
      if (currentPlayer) {
        monsterManager.update(deltaTime)
        playerDebugInfo.set({
          position: {
            x: currentPlayer.position.x,
            y: currentPlayer.position.y,
            z: currentPlayer.position.z,
          },
          rotation: currentPlayerState.rotation,
        })
      } else {
        playerDebugInfo.set(null)
      }
      loopProfiler.record('monsterLogic', performance.now() - monsterLogicStart)

      // Update housing (player-inside detection + front wall toggling).
      // Skipped underground so house-floor detection can't fight the
      // dungeon height provider.
      if (!$isUnderground) {
        const housingStart = performance.now()
        housingLayerRef?.update(deltaTime)
        loopProfiler.record('housingUpdate', performance.now() - housingStart)
      }

      // Dungeon stair-shaft floor transitions
      if (currentPlayer) {
        dungeonLayerRef?.update(
          currentPlayer.position.x,
          currentPlayer.position.y,
          currentPlayer.position.z,
          deltaTime
        )
      }

      // Update tree occlusion (hide trees that block camera view of player)
      treeLayerRef?.update()

      // Update ground items (spin animation)
      groundItemsLayerRef?.update(deltaTime)

      // Update grass wind & trail
      {
        const grassStart = performance.now()
        grassLayerRef?.update(deltaTime, renderer)
        loopProfiler.record('grassUpdate', performance.now() - grassStart)
      }

      // Update wind-blown particles (only when grass is visible nearby)
      {
        const windStart = performance.now()
        const windState = grassLayerRef?.getWindState()
        const grassCount = grassLayerRef?.getPlayerChunkGrassCount() ?? 0
        if (windState)
          windParticlesRef?.update(deltaTime, camera, windState, grassCount)
        loopProfiler.record('windParticles', performance.now() - windStart)
      }

      // Update river-rock spray particles + wake scroll
      riverRocksRef?.update(deltaTime, camera)

      // Update camera with preserved offset
      const cameraUpdateStart = performance.now()
      updateCameraWithOffset(cameraOffset)
      loopProfiler.record('cameraUpdate', performance.now() - cameraUpdateStart)

      // Compute sun snapshot once per frame (reused by lighting + water)
      const calDate = calendarSystem.getDate()
      const sunSnapshot = computeSunLightSnapshot(displayHour, calDate)

      // Update directional light to follow player
      const lightUpdateStart = performance.now()
      updateLightPosition(sunSnapshot, calDate)
      loopProfiler.record('lightUpdate', performance.now() - lightUpdateStart)

      // Update water uniforms — always use real sun direction (not moon)
      waterTime += realDeltaSeconds
      tickWavePhase(waterTime)
      // Shore-spray pulses ride the same swell clock as the water shader.
      shoreSprayRef?.update(deltaTime, camera, waterTime)
      waterSunDirTmp.set(
        sunSnapshot.direction.x,
        sunSnapshot.direction.y,
        sunSnapshot.direction.z
      )
      waterSunDir = waterSunDirTmp.clone()
      if (directionalLight) {
        waterSunColor = directionalLight.color.clone()
        // Moon brightness: use directional light intensity when sun is below horizon
        waterMoonBrightness =
          waterSunDirTmp.y <= 0 ? directionalLight.intensity : 0
      }
      if (camera) {
        camera.getWorldDirection(waterCamDirTmp)
        waterCamDir = waterCamDirTmp.clone()
      }

      runRenderPasses({
        renderer,
        camera,
        multiPassRenderer,
        refractionManager,
        reflectionManager,
        refractionEnabled: $refractionEnabled && !$isUnderground,
        reflectionEnabled: $reflectionEnabled && !$isUnderground,
        hasWater: waterLayerRef?.hasActiveWater() ?? false,
        waterGroup,
        terrainGroup,
        terrainMeshes,
        entityClipGroup,
        waterLayerRef,
        riverRocksRef,
        shoreSprayRef,
        grassLayerRef,
        treeLayerRef,
        windParticlesRef,
        housingLayerRef,
        objectOverlayRef,
        currentPlayerModel,
        otherPlayerModels,
        monsterModels,
        renderProfiler,
        loopProfiler,
      })

      loopProfiler.record('frameWork', performance.now() - frameWorkStart)

      // Request the main scene render now that this frame's state and water
      // render targets are current. The canvas is in manual mode, so this is
      // the only thing that draws it during gameplay.
      if (renderCadence.shouldRender(fixedDeltaTime)) invalidate()

      // `main` captures Threlte's automatic main scene render on its own rAF;
      // 'refraction'/'reflection'/'wetness' come from passes tagged via withTag.
      recordRenderProfilerStats(renderProfiler, loopProfiler)
      renderProfiler.resetFrame()

      // Detect when pipeline compilation is done: once data is ready,
      // wait for a few consecutive smooth frames before hiding the loading dialog.
      if (isSceneCompiling && initialDataReady) {
        if (rawDeltaTime < SMOOTH_FRAME_TIME_MS) {
          smoothFrameCount++
          if (smoothFrameCount >= SMOOTH_FRAME_THRESHOLD) {
            isSceneCompiling = false
          }
        } else {
          smoothFrameCount = 0
        }
      }

      if (unclampedSteps > MAX_CATCH_UP_STEPS) {
        // Drop excessive backlog after long stalls (tab switch, debugger pause, etc.).
        lastFrameTime = currentTime - FRAME_TIME_MS
      } else {
        lastFrameTime += fixedDeltaTime
      }
    }

    loopProfiler.flush(currentTime)

    // Always continue the loop
    gameLoopId = requestAnimationFrame(gameLoop)
  }

  function calculateCameraOffset() {
    const offset = getCameraOffsetFromScene(
      camera,
      currentPlayer?.position ?? null,
      CAMERA_OFFSET
    )
    if (camera) {
      // Update camera "zoom metric" for debug UI.
      cameraDistance.set(camera.zoom)
    }
    return offset
  }

  function updateCameraWithOffset(offset: { x: number; y: number; z: number }) {
    if (!currentPlayer || !camera) return

    if ($mapEditorMode || $housingEditorMode) {
      // Apply editor pan offset so middle-mouse drag moves the viewport
      const pan = $editorPanOffset
      const panPos = {
        x: currentPlayer.position.x + pan.x,
        y: currentPlayer.position.y,
        z: currentPlayer.position.z + pan.z,
      }
      // Always use fixed CAMERA_OFFSET in editor mode (OrbitControls is disabled,
      // so we must not feed back the computed offset which includes the pan).
      if (camera.zoom < 1) {
        const maxBelow = INITIAL_DISTANCE / Math.SQRT2
        const scale = Math.max(
          1,
          ORTHOGRAPHIC_FRUSTUM_HEIGHT / 2 / (camera.zoom * maxBelow)
        )
        cameraTarget = applyCameraOffset(camera, panPos, {
          x: CAMERA_OFFSET.x * scale,
          y: CAMERA_OFFSET.y * scale,
          z: CAMERA_OFFSET.z * scale,
        })
      } else {
        cameraTarget = applyCameraOffset(camera, panPos, CAMERA_OFFSET)
      }
    } else {
      cameraTarget = applyCameraOffset(camera, currentPlayer.position, offset)
    }
  }

  function resetCameraToInitialState() {
    if (!currentPlayer || !camera) return
    cameraTarget = resetCameraToDefaultState(
      camera,
      currentPlayer.position,
      CAMERA_OFFSET
    )
    cameraDistance.set(camera.zoom)
  }

  function updateLightPosition(
    sunLightSnapshot: SunLightSnapshot,
    calDate: CalendarDate
  ) {
    sceneLighting.update({
      currentPlayerPosition: currentPlayer?.position ?? null,
      localCalendarDate: calDate,
      ambientLight,
      directionalLight,
      directionalShadowsEnabled: graphicsPreset.enableDirectionalShadows,
      scene,
      sunLightSnapshot,
      eclipseFactor: eclipseState.factor,
      underground: $isUnderground,
    })
  }

  // Stop game loop
  function stopGameLoop() {
    if (gameLoopId !== null) {
      cancelAnimationFrame(gameLoopId)
      gameLoopId = null
    }
  }

  onMount(() => {
    loopProfileEnabled = false
    loopProfiler.resetWindow(performance.now())
    renderProfiler.wrap(renderer)

    const cleanupDebugConsole = registerDebugConsole(() => ({
      loopProfiler,
      getLoopProfileEnabled: () => loopProfileEnabled,
      setLoopProfileEnabled: (v) => {
        loopProfileEnabled = v
      },
      renderer,
      scene,
      getGrassGroup: () => grassLayerRef?.getGroup(),
      getHousingGroup: () => housingLayerRef?.getGroup(),
      getTerrainGroup: () => terrainGroup,
      refractionEnabled,
      reflectionEnabled,
      splatManager: terrainSplatManager,
      getPlayerPos: () =>
        currentPlayer
          ? { x: currentPlayer.position.x, z: currentPlayer.position.z }
          : null,
    }))
    {
      const initHour = calendarSystem.getGameHour()
      setGameHour(initHour + $sunDebugOffset, initHour)
    }

    const unsubscribeServerGameTime = serverGameTime.subscribe((gameTime) => {
      latestServerGameTime = gameTime
      calendarSystem.applyServerTimeIfAllowed(
        latestServerGameTime,
        latestSunTimeScale
      )
    })

    const unsubscribeSunTimeScale = sunTimeScale.subscribe((scale) => {
      const wasFastSun = latestSunTimeScale > 1
      latestSunTimeScale = scale
      if (wasFastSun && scale <= 1) {
        calendarSystem.applyServerTimeIfAllowed(
          latestServerGameTime,
          latestSunTimeScale
        )
      }
    })

    const unsubscribeViewportSize = size.subscribe((nextSize) => {
      viewportSize = nextSize
      // Keep refraction/reflection targets stable across viewport changes.
      // Chrome's debugging banner changes the viewport height when it attaches
      // or detaches; resizing RenderTarget disposes its GPU texture, and
      // three.js WebGPU can render a transparent material once with a stale
      // bind group before the replacement texture is visible to all bindings.
    })

    const unsubscribeCameraReset = cameraResetNonce.subscribe((nonce) => {
      // Ignore initial store emission; only react to explicit reset requests.
      if (nonce > 0) {
        resetCameraToInitialState()
      }
    })

    const sceneRes = initScene(
      renderer,
      scene,
      viewportSize.width,
      viewportSize.height,
      {
        skipWaterLayer: !graphicsPreset.enableWaterLayer,
        skipWaterEffects: !graphicsPreset.enableWaterEffects,
        waterRtDivisor: graphicsPreset.waterRtDivisor,
      }
    )
    terrainGeometry = sceneRes.terrainGeometry
    waterNormalMap = sceneRes.waterNormalMap
    sceneRes.waterFoamMapPromise.then((tex) => {
      waterFoamMap = tex
    })
    sceneRes.waterCausticsMapPromise.then((tex) => {
      waterCausticsMap = tex
    })
    refractionManager = sceneRes.refractionManager
    refractionTexture = sceneRes.refractionTexture
    reflectionManager = sceneRes.reflectionManager
    reflectionTexture = sceneRes.reflectionTexture

    // Desktop loads all terrain tiles immediately. Mobile staggers the initial
    // tile mount to avoid WebGPU memory spikes while the world is preparing.
    tileManager.rebuild(terrainCenterChunk.x, terrainCenterChunk.z)
    if (graphicsPreset.initialTerrainQueueDrainCount === Infinity) {
      tileManager.drainAll()
    } else {
      for (let i = 0; i < graphicsPreset.initialTerrainQueueDrainCount; i++) {
        tileManager.drainQueue()
      }
    }

    bootstrapSceneAssets({
      renderer,
      terrainTiles,
      heightManager: terrainHeightManager,
      playerPosition: currentPlayer?.position ?? null,
      grassLayerRef: graphicsPreset.enableGrassLayer
        ? grassLayerRef
        : undefined,
      housingLayerRef: graphicsPreset.enableHousingLayer
        ? housingLayerRef
        : undefined,
      graphicsPreset,
    }).then(() => {
      // Mark data as ready. Threlte's render loop compiles WebGPU pipelines
      // on-demand (synchronously per frame) under the loading dialog overlay.
      // The smooth frame detector waits until compilation is done.
      initialDataReady = true
    })

    // Start game loop
    lastFrameTime = performance.now()
    initFpsCounting()
    gameLoopId = requestAnimationFrame(gameLoop)

    // Start chat bubble expiration checker
    startChatBubbleChecker()

    networkManager.connect(serverUrl)

    // Initialize camera position: HMR restores previous state,
    // otherwise a reactive $effect waits for camera + player to be ready.
    if (_hmrCameraInitialized) {
      cameraInitialized = true
      if (_hmrCameraZoom !== null) {
        const restoreZoom = _hmrCameraZoom
        requestAnimationFrame(() => {
          if (camera) {
            camera.zoom = restoreZoom
            camera.updateProjectionMatrix()
          }
        })
      }
      _hmrCameraZoom = null
    }

    return () => {
      cleanupDebugConsole()
      scene.environment?.dispose()
      scene.environment = null
      unsubscribeViewportSize()
      unsubscribeCameraReset()
      unsubscribeServerGameTime()
      unsubscribeSunTimeScale()
      stopGameLoop()
      stopChatBubbleChecker()
      networkManager.disconnect()
      monsterManager.reset()
      remotePlayerManager.reset()
      playerDebugInfo.set(null)
      terrainHeightManager.destroy()
      terrainSplatManager.destroy()
      waterNormalMap?.dispose()
      waterNormalMap = null
      waterFoamMap?.dispose()
      waterFoamMap = null
      waterCausticsMap?.dispose()
      waterCausticsMap = null
      refractionManager?.dispose()
      refractionManager = null
      refractionTexture = null
      reflectionManager?.dispose()
      reflectionManager = null
      reflectionTexture = null
      terrainTiles = []
      terrainMeshes = []
      resetGameStore()
    }
  })
</script>

<T.OrthographicCamera
  bind:ref={camera}
  makeDefault
  zoom={ORTHOGRAPHIC_DEFAULT_ZOOM}
>
  <OrbitControls
    enableRotate={$mapEditorMode || $housingEditorMode
      ? false
      : $cameraRotationEnabled}
    enablePan={false}
    enableZoom={!$mapEditorMode && !$housingEditorMode}
    enabled={!$mapEditorMode && !$housingEditorMode}
    target={cameraTarget}
    minZoom={$debugSpeedMode ? 0.15 : 1}
    maxZoom={2}
  />
</T.OrthographicCamera>

<T.DirectionalLight
  bind:ref={directionalLight}
  position={[10, 10, 10]}
  intensity={SUN_MAX_INTENSITY}
  castShadow={graphicsPreset.enableDirectionalShadows}
  shadow.bias={-0.0002}
  shadow.normalBias={0.15}
  shadow.mapSize.width={graphicsPreset.shadowMapSize}
  shadow.mapSize.height={graphicsPreset.shadowMapSize}
/>
<T.AmbientLight
  bind:ref={ambientLight}
  intensity={sceneLighting.ambientDayIntensity}
  color="#ffffff"
/>
<!-- Placeholder shadow-casting PointLight so WebGPU compiles ALL material
     pipelines with point-light shadow support from the start. Without this,
     adding the player's torch later triggers a cascade recompilation of every
     existing material (~12s stall). Intensity 0 = invisible but pipelines
     are compiled with shadow support. After compilation, move offscreen so
     the shadow frustum captures nothing (avoids 6× cube-face renders/frame). -->
{#if graphicsPreset.enableTorchShadows}
  <T.PointLight
    bind:ref={placeholderShadowLight}
    position={isSceneCompiling ? [0, 0, 0] : [0, OFFSCREEN_Y, 0]}
    intensity={0}
    distance={50}
    decay={1.2}
    castShadow
    shadow.mapSize.width={graphicsPreset.torchShadowMapSize}
    shadow.mapSize.height={graphicsPreset.torchShadowMapSize}
    shadow.camera.near={0.5}
    shadow.camera.far={50}
  />
{/if}

<GameSceneTerrainLayer
  {terrainGeometry}
  {terrainTiles}
  bind:terrainMeshes
  bind:terrainGroup
  bind:syncTileMeshes
  heightManager={terrainHeightManager}
  splatManager={terrainSplatManager}
  terrainMaterialPrecompilePoolSize={graphicsPreset.terrainMaterialPrecompilePoolSize}
  {renderer}
  {camera}
/>

{#if graphicsPreset.enableHousingLayer}
  <GameSceneHousingLayer
    bind:this={housingLayerRef}
    playerPosition={currentPlayer?.position ?? null}
  />
{/if}

<GameSceneDungeonLayer
  bind:this={dungeonLayerRef}
  onPropReady={(entranceId, depth, propId, x, z) =>
    playerControl?.swingAndBreakProp(entranceId, depth, propId, x, z)}
/>

{#if graphicsPreset.enableGrassLayer}
  <GameSceneGrassLayer
    bind:this={grassLayerRef}
    {terrainTiles}
    grassDataManager={terrainGrassDataManager}
    playerPosition={currentPlayer?.position ?? null}
    grassCastsShadow={graphicsPreset.grassCastsShadow}
  />
{/if}

{#if graphicsPreset.enableTreeLayer}
  <GameSceneTreeLayer
    bind:this={treeLayerRef}
    {terrainTiles}
    treeDataManager={terrainTreeDataManager}
    playerPosition={currentPlayer?.position ?? null}
    maxInstances={graphicsPreset.treeInstanceLimit}
    treeCastsShadow={graphicsPreset.treeCastsShadow}
  />
{/if}

{#if graphicsPreset.enableWindParticles}
  <GameSceneWindParticles
    bind:this={windParticlesRef}
    playerPosition={currentPlayer?.position ?? null}
  />
{/if}

{#if graphicsPreset.enableWaterLayer}
  <GameSceneWaterFieldLayer
    bind:this={waterLayerRef}
    {terrainTiles}
    heightManager={terrainHeightManager}
    {waterFieldManager}
    normalMap={waterNormalMap}
    foamMap={waterFoamMap}
    causticsMap={waterCausticsMap}
    time={waterTime}
    sunDirection={waterSunDir}
    sunColor={waterSunColor}
    cameraDirection={waterCamDir}
    moonBrightness={waterMoonBrightness}
    refractionMap={refractionTexture}
    reflectionMap={reflectionTexture}
    torchLight={waterTorchLight}
    bind:waterGroup
  />
  <GameSceneRiverRocksLayer
    bind:this={riverRocksRef}
    {terrainTiles}
    {waterFieldManager}
    heightManager={terrainHeightManager}
    foamMap={waterFoamMap}
    sunDirection={waterSunDir}
    playerPosition={currentPlayer?.position ?? null}
    objectPlacements={$currentObjectData.placements}
  />
  <GameSceneShoreSprayLayer
    bind:this={shoreSprayRef}
    {terrainTiles}
    {waterFieldManager}
    heightManager={terrainHeightManager}
    foamMap={waterFoamMap}
    sunDirection={waterSunDir}
    playerPosition={currentPlayer?.position ?? null}
  />
{/if}

<T is={entityClipGroupObj} bind:ref={entityClipGroup}>
  <GameScenePlayersLayer
    bind:this={playersLayer}
    {camera}
    {cameraInitialized}
    {currentPlayer}
    {otherPlayers}
    remotePlayers={remotePlayerManager.players}
    {chatBubbles}
    {currentPlayerState}
    {terrainMeshes}
    housingGroup={housingLayerRef?.getGroup() ?? null}
    dungeonGroup={dungeonLayerRef?.getFloorGroup() ?? null}
    doorMeshes={[
      ...(housingLayerRef?.getDoorMeshes() ?? []),
      ...(dungeonLayerRef?.getDoorMeshes() ?? []),
    ]}
    objectMeshes={objectOverlayRef ? [objectOverlayRef.getGroup()] : []}
    propMeshes={dungeonLayerRef?.getPropMeshes() ?? []}
    groundItemMeshes={groundItemsLayerRef?.getGroup()
      ? [groundItemsLayerRef.getGroup()!]
      : []}
    {monsterModels}
    {playerAttackDuration}
    torchEffectsDisabled={!graphicsPreset.enableTorchEffects}
    torchLightCastsShadow={graphicsPreset.enableTorchShadows}
    torchShadowMapSize={graphicsPreset.torchShadowMapSize}
    wallTorchPositions={() => dungeonLayerRef?.getWallTorchPositions() ?? []}
    heightManager={terrainHeightManager}
    waterSurfaceAt={(x, z) => waterFieldManager.surfaceAt(x, z)}
    onStateChange={handlePlayerStateChange}
    onPlayerControlEvent={enqueuePlayerControlEvent}
    onAttackDuration={(duration) => (playerAttackDuration = duration)}
    {onCurrentPlayerDyingFinished}
    bind:isCurrentPlayerLoading
    bind:playerControl
    bind:currentPlayerModel
    bind:otherPlayerModels
  />

  <GameSceneMonstersLayer
    monsters={monsterManager.monsters}
    bind:monsterModels
  />

  <GameSceneGroundItemsLayer
    bind:this={groundItemsLayerRef}
    heightManager={terrainHeightManager}
  />

  {#each [...$fishingBobbers] as [playerId, bobber] (playerId)}
    <FishingBobber {bobber} />
  {/each}
</T>

{#if $mapEditorMode}
  <MapEditorCursor
    {camera}
    {terrainMeshes}
    {terrainTiles}
    heightManager={terrainHeightManager}
    splatManager={terrainSplatManager}
    grassDataManager={terrainGrassDataManager}
    getObjectGroup={() => objectOverlayRef?.getGroup() ?? null}
  />
  <ZoneOverlay />
  <RoadOverlay />
  <NpcWaypointOverlay />
{/if}
<ObjectOverlay bind:this={objectOverlayRef} />

{#if $hoveredSignpost && camera}
  <SignpostBubble
    bind:this={signpostBubbleRef}
    position={signpostBubblePos}
    {camera}
    message={$hoveredSignpost.text}
  />
{/if}

{#if $housingEditorMode}
  <HousingEditorCursor
    {camera}
    {terrainMeshes}
    heightManager={terrainHeightManager}
    grassDataManager={terrainGrassDataManager}
    housingGroup={housingLayerRef?.getGroup() ?? null}
  />
{/if}

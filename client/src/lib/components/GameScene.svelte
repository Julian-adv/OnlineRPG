<script lang="ts">
  import { T } from '@threlte/core'
  import { OrbitControls, Grid } from '@threlte/extras'
  import * as THREE from 'three'
  import { onMount } from 'svelte'
  import { gameStore, type Player } from '../stores/gameStore'
  import { networkManager } from '../network/socket'
  import PlayerModel from './PlayerModel.svelte'
  import PlayerControl, { type PlayerState } from './PlayerControl.svelte'
  import InstancedGrass from './InstancedGrass.svelte'

  let currentPlayer = $state<Player | null>(null)
  let otherPlayers = $state(new Map())
  let camera = $state<THREE.PerspectiveCamera | undefined>(undefined)
  let groundMesh = $state<THREE.Mesh | undefined>(undefined)
  let cameraInitialized = $state(false)

  // Camera follow system
  let cameraTarget = $state<[number, number, number]>([0, 0, 0])
  const CAMERA_OFFSET = { x: 0, y: 5, z: 5 } // Relative to player

  // Game loop
  let gameLoopId = $state<number | null>(null)
  let lastFrameTime = $state(0)
  const TARGET_FPS = 120
  const FRAME_TIME = 1000 / TARGET_FPS // 16.67ms

  // Player state from PlayerControl
  let currentPlayerState = $state<PlayerState>({
    state: 'idle',
    speed: 0,
    direction: 0,
    position: { x: 0, y: 0, z: 0 }
  })

  // Reference to InstancedGrass component
  let instancedGrass: InstancedGrass
  
  // References to PlayerModel components
  let currentPlayerModel = $state<PlayerModel | null>(null)
  let otherPlayerModels = $state<PlayerModel[]>([])
  
  // Reference to PlayerControl component
  let playerControl: PlayerControl

  // Handle player state changes from PlayerControl
  function handlePlayerStateChange(newState: PlayerState) {
    currentPlayerState = newState
  }

  gameStore.subscribe((state) => {
    currentPlayer = state.currentPlayer
    otherPlayers = state.otherPlayers
  })

  // Main game loop with 60fps throttling
  function gameLoop(currentTime: number) {
    const deltaTime = currentTime - lastFrameTime

    // Throttle to 60fps
    if (deltaTime >= FRAME_TIME) {
      // Calculate camera offset before player movement
      const cameraOffset = calculateCameraOffset()

      // Update player controls
      if (playerControl) {
        playerControl.updateKeyboardMovement()
        playerControl.updatePlayerMovement(currentTime)
      }

      // Update animated grass
      if (instancedGrass) {
        instancedGrass.updateMixer(deltaTime)
      }
      
      // Update player model animations
      if (currentPlayerModel) {
        currentPlayerModel.updateMixer()
        currentPlayerModel.updateAnimationState()
      }
      
      // Update other player model animations
      for (const playerModel of otherPlayerModels) {
        playerModel.updateMixer()
        playerModel.updateAnimationState()
      }

      // Update camera with preserved offset
      updateCameraWithOffset(cameraOffset)

      lastFrameTime = currentTime
    }

    // Always continue the loop
    gameLoopId = requestAnimationFrame(gameLoop)
  }


  function calculateCameraOffset() {
    if (!currentPlayer || !camera) {
      return { x: CAMERA_OFFSET.x, y: CAMERA_OFFSET.y, z: CAMERA_OFFSET.z }
    }

    // Calculate current distance vector from player to camera
    const currentCameraPos = camera.position
    const playerPos = currentPlayer.position

    // Get the current distance vector (preserving zoom)
    const distanceVector = {
      x: currentCameraPos.x - playerPos.x,
      y: currentCameraPos.y - playerPos.y,
      z: currentCameraPos.z - playerPos.z,
    }

    return distanceVector
  }

  function updateCameraWithOffset(offset: { x: number; y: number; z: number }) {
    if (!currentPlayer || !camera) return

    const playerPos = currentPlayer.position

    // Update camera position by adding the preserved offset to new player position
    const newCameraPosition = {
      x: playerPos.x + offset.x,
      y: playerPos.y + offset.y,
      z: playerPos.z + offset.z,
    }

    camera.position.set(
      newCameraPosition.x,
      newCameraPosition.y,
      newCameraPosition.z
    )

    // Make camera look at player directly
    camera.lookAt(playerPos.x, playerPos.y, playerPos.z)

    // Update camera target to look at player
    cameraTarget = [playerPos.x, playerPos.y, playerPos.z]
  }


  // Stop game loop
  function stopGameLoop() {
    if (gameLoopId !== null) {
      cancelAnimationFrame(gameLoopId)
      gameLoopId = null
    }
  }

  onMount(() => {
    // Start game loop
    lastFrameTime = performance.now()
    gameLoopId = requestAnimationFrame(gameLoop)

    networkManager.connect()

    // Join the game with a default player name
    setTimeout(() => {
      networkManager.joinGame('Player')
    }, 1000)

    // Initialize camera position after a short delay to ensure camera ref is available
    setTimeout(() => {
      if (camera && currentPlayer) {
        // Set initial camera position
        camera.position.set(
          currentPlayer.position.x + CAMERA_OFFSET.x,
          currentPlayer.position.y + CAMERA_OFFSET.y,
          currentPlayer.position.z + CAMERA_OFFSET.z
        )
        cameraInitialized = true

        // Make camera look at player directly
        camera.lookAt(
          currentPlayer.position.x,
          currentPlayer.position.y,
          currentPlayer.position.z
        )

        // Set initial camera target to look at player
        cameraTarget = [
          currentPlayer.position.x,
          currentPlayer.position.y,
          currentPlayer.position.z,
        ]
      }
    }, 1100)

    return () => {
      stopGameLoop()
      networkManager.disconnect()
    }
  })

</script>

<T.PerspectiveCamera bind:ref={camera} makeDefault fov={75}>
  <OrbitControls
    enableRotate={true}
    enablePan={false}
    enableZoom={true}
    target={cameraTarget}
    minDistance={5}
    maxDistance={20}
  />
</T.PerspectiveCamera>

<T.DirectionalLight position={[10, 10, 10]} intensity={1.5} castShadow />
<T.AmbientLight intensity={0.4} />

<Grid
  infiniteGrid
  gridSize={100}
  sectionColor="#4a5568"
  sectionThickness={1.2}
  fadeDistance={100}
  position={[0, 0.1, 0]}
/>

<T.Mesh
  bind:ref={groundMesh}
  position={[0, 0, 0]}
  rotation={[-Math.PI / 2, 0, 0]}
  receiveShadow
>
  <T.PlaneGeometry args={[100, 100]} />
  <T.MeshLambertMaterial color="#4a7c59" />
</T.Mesh>

<InstancedGrass bind:this={instancedGrass} />

<!-- PlayerControl component handles input and updates player state -->
<PlayerControl 
  bind:this={playerControl} 
  onStateChange={handlePlayerStateChange}
  {camera}
  {groundMesh}
/>

{#if currentPlayer && cameraInitialized}
  <PlayerModel
    bind:this={currentPlayerModel}
    position={currentPlayer.position}
    name={currentPlayer.name}
    isCurrentPlayer={true}
    playerState={currentPlayerState.state}
    speed={currentPlayerState.speed}
    rotation={currentPlayerState.direction}
    cameraPosition={camera?.position}
  />
{/if}

{#if cameraInitialized}
  {#each [...otherPlayers.values()] as player, index (player.id)}
    <PlayerModel
      bind:this={otherPlayerModels[index]}
      position={player.position}
      name={player.name}
      isCurrentPlayer={false}
      cameraPosition={camera?.position}
    />
  {/each}
{/if}

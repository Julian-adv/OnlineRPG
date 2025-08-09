<script lang="ts">
  import { T } from '@threlte/core'
  import { OrbitControls, Grid } from '@threlte/extras'
  import { Vector2, Raycaster } from 'three'
  import type * as THREE from 'three'
  import { onMount } from 'svelte'
  import { gameStore, type Player } from '../stores/gameStore'
  import { networkManager } from '../network/socket'
  import PlayerModel from './PlayerModel.svelte'

  let currentPlayer = $state<Player | null>(null)
  let otherPlayers = $state(new Map())
  let camera = $state<THREE.PerspectiveCamera | undefined>(undefined)
  let groundMesh = $state<THREE.Mesh | undefined>(undefined)

  // Movement system
  let movementTarget = $state<{ x: number; y: number; z: number } | null>(null)
  let isMoving = $state(false)
  let movementStartTime = $state(0)
  let movementStartPosition = $state<{
    x: number
    y: number
    z: number
  } | null>(null)
  const MOVEMENT_SPEED = 3 // units per second

  // Camera follow system
  let cameraTarget = $state<[number, number, number]>([0, 0, 0])
  const CAMERA_OFFSET = { x: 0, y: 15, z: 10 } // Relative to player

  // Game loop
  let gameLoopId = $state<number | null>(null)
  let lastFrameTime = $state(0)
  const TARGET_FPS = 60
  const FRAME_TIME = 1000 / TARGET_FPS // 16.67ms

  gameStore.subscribe((state) => {
    currentPlayer = state.currentPlayer
    otherPlayers = state.otherPlayers
  })

  // Main game loop with 60fps throttling
  function gameLoop(currentTime: number) {
    const deltaTime = currentTime - lastFrameTime

    // Throttle to 60fps
    if (deltaTime >= FRAME_TIME) {
      // Update player movement
      updatePlayerMovement(currentTime)

      // Always update camera
      updateCamera()

      lastFrameTime = currentTime
    }

    // Always continue the loop
    gameLoopId = requestAnimationFrame(gameLoop)
  }

  function updatePlayerMovement(currentTime: number) {
    if (
      !isMoving ||
      !movementTarget ||
      !currentPlayer ||
      !movementStartPosition
    ) {
      return
    }

    const elapsed = currentTime - movementStartTime
    const dx = movementTarget.x - movementStartPosition.x
    const dz = movementTarget.z - movementStartPosition.z
    const distance = Math.sqrt(dx * dx + dz * dz)
    const duration = (distance / MOVEMENT_SPEED) * 1000 // Convert to milliseconds

    const progress = Math.min(elapsed / duration, 1)

    if (progress < 1) {
      // Linear interpolation
      const newX = movementStartPosition.x + dx * progress
      const newZ = movementStartPosition.z + dz * progress

      gameStore.update((state) => {
        if (state.currentPlayer) {
          state.currentPlayer.position.set(newX, movementTarget!.y, newZ)
        }
        return state
      })
    } else {
      // Movement complete
      gameStore.update((state) => {
        if (state.currentPlayer && movementTarget) {
          state.currentPlayer.position.set(
            movementTarget.x,
            movementTarget.y,
            movementTarget.z
          )
        }
        return state
      })

      // Send final position to server
      networkManager.sendPlayerMove(movementTarget)

      isMoving = false
      movementTarget = null
      movementStartPosition = null
    }
  }

  function updateCamera() {
    if (!currentPlayer || !camera) return

    // Update camera position to follow player with offset
    const newCameraPosition = {
      x: currentPlayer.position.x + CAMERA_OFFSET.x,
      y: currentPlayer.position.y + CAMERA_OFFSET.y,
      z: currentPlayer.position.z + CAMERA_OFFSET.z,
    }

    camera.position.set(
      newCameraPosition.x,
      newCameraPosition.y,
      newCameraPosition.z
    )

    // Update camera target to look at player
    cameraTarget = [
      currentPlayer.position.x,
      currentPlayer.position.y,
      currentPlayer.position.z,
    ]
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

    // Add click event listener to canvas - wait until canvas exists
    let canvas: HTMLCanvasElement | null = null
    const findCanvas = () => {
      canvas = document.querySelector('canvas')
      if (canvas) {
        canvas.addEventListener('mousedown', handleCanvasClick)
      } else {
        setTimeout(findCanvas, 100)
      }
    }
    findCanvas()

    return () => {
      stopGameLoop()
      networkManager.disconnect()
      if (canvas) {
        canvas.removeEventListener('click', handleCanvasClick)
      }
    }
  })

  function handlePlayerMove(detail: { x: number; y: number; z: number }) {
    const { x, y, z } = detail
    networkManager.sendPlayerMove({ x, y, z })
  }

  function handleCanvasClick(event: MouseEvent) {
    if (!camera || !groundMesh || !currentPlayer || isMoving) return

    // Calculate mouse position in normalized device coordinates (-1 to +1)
    const rect = (event.target as HTMLCanvasElement).getBoundingClientRect()
    const mouse = new Vector2(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1
    )

    // Create raycaster
    const raycaster = new Raycaster()
    raycaster.setFromCamera(mouse, camera)

    // Check intersection with ground
    const intersects = raycaster.intersectObject(groundMesh)

    if (intersects.length > 0) {
      const point = intersects[0].point
      const clickPosition = {
        x: point.x,
        y: 1, // Keep player above ground
        z: point.z,
      }

      // Set movement target and start moving
      movementTarget = clickPosition
      movementStartPosition = {
        x: currentPlayer.position.x,
        y: currentPlayer.position.y,
        z: currentPlayer.position.z,
      }
      movementStartTime = performance.now()
      isMoving = true
    }
  }
</script>

<T.PerspectiveCamera bind:ref={camera} makeDefault fov={75}>
  <OrbitControls
    enableRotate={false}
    enablePan={false}
    enableZoom={true}
    target={cameraTarget}
    minDistance={5}
    maxDistance={50}
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
/>

<T.Mesh
  bind:ref={groundMesh}
  position={[0, -0.5, 0]}
  rotation={[-Math.PI / 2, 0, 0]}
  receiveShadow
>
  <T.PlaneGeometry args={[100, 100]} />
  <T.MeshLambertMaterial color="#2d3748" />
</T.Mesh>

{#if currentPlayer}
  <PlayerModel
    position={currentPlayer.position}
    name={currentPlayer.name}
    isCurrentPlayer={true}
    onmove={handlePlayerMove}
  />
{/if}

{#each [...otherPlayers.values()] as player (player.id)}
  <PlayerModel
    position={player.position}
    name={player.name}
    isCurrentPlayer={false}
  />
{/each}

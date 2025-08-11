<script lang="ts">
  import { T, useLoader } from '@threlte/core'
  import { OrbitControls, Grid } from '@threlte/extras'
  import {
    Vector2,
    Raycaster,
    Matrix4,
    Object3D,
    Mesh,
    InstancedMesh,
  } from 'three'
  import type * as THREE from 'three'
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import { onMount } from 'svelte'
  import { gameStore, type Player } from '../stores/gameStore'
  import { networkManager } from '../network/socket'
  import PlayerModel from './PlayerModel.svelte'

  let currentPlayer = $state<Player | null>(null)
  let otherPlayers = $state(new Map())
  let camera = $state<THREE.PerspectiveCamera | undefined>(undefined)
  let groundMesh = $state<THREE.Mesh | undefined>(undefined)
  let cameraInitialized = $state(false)

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
  const CAMERA_OFFSET = { x: 0, y: 5, z: 5 } // Relative to player

  // Game loop
  let gameLoopId = $state<number | null>(null)
  let lastFrameTime = $state(0)
  const TARGET_FPS = 60
  const FRAME_TIME = 1000 / TARGET_FPS // 16.67ms

  // Keyboard controls
  let keysPressed = $state(new Set<string>())

  // Character rotation
  let playerRotation = $state(0)

  // InstancedMesh for grass
  let grassInstancedMeshes = $state<InstancedMesh[]>([])
  let grassMatrices = $state<Matrix4[][]>([])

  // Load individual grass models
  const grass1 = useLoader(GLTFLoader).load('/models/grass_1_Object_4.glb')
  const grass2 = useLoader(GLTFLoader).load('/models/grass_2_Object_6.glb')
  const grass3 = useLoader(GLTFLoader).load('/models/grass_3_Object_8.glb')
  const grass4 = useLoader(GLTFLoader).load('/models/grass_4_Object_10.glb')
  const grass5 = useLoader(GLTFLoader).load('/models/grass_5_Object_12.glb')
  const grass6 = useLoader(GLTFLoader).load('/models/grass_6_Object_14.glb')
  const grass7 = useLoader(GLTFLoader).load('/models/grass_7_Object_16.glb')
  const grass8 = useLoader(GLTFLoader).load('/models/grass_8_Object_18.glb')
  const grass9 = useLoader(GLTFLoader).load('/models/grass_9_Object_20.glb')

  // Group them for easy access
  const grassModels = [
    grass1,
    grass2,
    grass3,
    grass4,
    grass5,
    grass6,
    grass7,
    grass8,
    grass9,
  ]

  // Setup InstancedMesh when all models are loaded
  $effect(() => {
    const models = [
      { name: 'grass1', model: $grass1 },
      { name: 'grass2', model: $grass2 },
      { name: 'grass3', model: $grass3 },
      { name: 'grass4', model: $grass4 },
      { name: 'grass5', model: $grass5 },
      { name: 'grass6', model: $grass6 },
      { name: 'grass7', model: $grass7 },
      { name: 'grass8', model: $grass8 },
      { name: 'grass9', model: $grass9 },
    ]

    const loadedModels = models.filter((m) => m.model)
    const loadedCount = loadedModels.length

    console.log(`${loadedCount}/9 grass models loaded`)

    if (loadedCount === 9) {
      setupInstancedGrass()
    }
  })

  function setupInstancedGrass() {
    const instancedMeshes: InstancedMesh[] = []
    const matrices: Matrix4[][] = []

    // Group positions by grass type
    const positionsByType: any[][] = Array(9)
      .fill(null)
      .map(() => [])
    grassPositions.forEach((pos) => {
      positionsByType[pos.grassType].push(pos)
    })

    // Create InstancedMesh for each grass type
    const grassModels = [
      $grass1,
      $grass2,
      $grass3,
      $grass4,
      $grass5,
      $grass6,
      $grass7,
      $grass8,
      $grass9,
    ]

    grassModels.forEach((model, index) => {
      if (model && positionsByType[index].length > 0) {
        const positions = positionsByType[index]
        const count = positions.length

        // Get the first mesh from the GLTF model
        let foundMesh: Mesh | null = null
        model.scene.traverse((child) => {
          if (child instanceof Mesh && !foundMesh) {
            foundMesh = child
          }
        })

        if (foundMesh) {
          // Create InstancedMesh with explicit type assertion
          const mesh = foundMesh as Mesh
          const instancedMesh = new InstancedMesh(
            mesh.geometry,
            mesh.material,
            count
          )

          instancedMesh.castShadow = true
          instancedMesh.receiveShadow = true

          // Create matrices for each instance
          const instanceMatrices: Matrix4[] = []
          const dummy = new Object3D()

          positions.forEach((pos, i) => {
            dummy.position.set(pos.x, 0.15, pos.z)
            dummy.rotation.set(0, pos.rotation, 0)
            dummy.scale.setScalar(pos.scale)
            dummy.updateMatrix()

            instancedMesh.setMatrixAt(i, dummy.matrix)
            instanceMatrices.push(dummy.matrix.clone())
          })

          instancedMesh.instanceMatrix.needsUpdate = true

          instancedMeshes.push(instancedMesh)
          matrices.push(instanceMatrices)
        }
      }
    })

    grassInstancedMeshes = instancedMeshes
    grassMatrices = matrices
    console.log(`Created ${instancedMeshes.length} InstancedMesh objects`)
  }

  // Generate grass positions with variety (simplified for testing)
  function generateGrassPositions() {
    const positions = []
    const spacing = 0.2 // Reasonable spacing for dense grass
    const gridSize = 15 // Smaller area for better performance
    let id = 0

    for (let x = -gridSize; x <= gridSize; x += spacing) {
      for (let z = -gridSize; z <= gridSize; z += spacing) {
        // Add some random offset for more natural look
        const offsetX = (Math.random() - 0.5) * 0.8
        const offsetZ = (Math.random() - 0.5) * 0.8
        const rotation = Math.random() * Math.PI * 2
        const scale = 1.4 + Math.random() * 0.8 // Random scale between 1.2 and 2.0

        positions.push({
          id: id++,
          x: x + offsetX,
          y: 0,
          z: z + offsetZ,
          rotation: rotation,
          grassType: Math.floor(Math.random() * 9), // Random grass type (0-8)
          scale: scale,
        })
      }
    }

    console.log(`Generated ${positions.length} grass positions`)
    console.log('First few positions:', positions.slice(0, 5))
    return positions
  }

  const grassPositions = generateGrassPositions()

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

      // Update keyboard movement
      updateKeyboardMovement()

      // Update player movement (click-to-move)
      updatePlayerMovement(currentTime)

      // Update camera with preserved offset
      updateCameraWithOffset(cameraOffset)

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

  // Keyboard movement system
  function updateKeyboardMovement() {
    if (!currentPlayer || keysPressed.size === 0) {
      return
    }

    // Cancel click-to-move if keyboard input detected
    if (keysPressed.size > 0 && movementTarget) {
      movementTarget = null
      movementStartPosition = null
      // isMoving will be set by keyboard movement below
    }

    // Calculate movement direction based on pressed keys
    let moveX = 0
    let moveZ = 0

    if (keysPressed.has('KeyW') || keysPressed.has('ArrowUp')) moveZ -= 1
    if (keysPressed.has('KeyS') || keysPressed.has('ArrowDown')) moveZ += 1
    if (keysPressed.has('KeyA') || keysPressed.has('ArrowLeft')) moveX -= 1
    if (keysPressed.has('KeyD') || keysPressed.has('ArrowRight')) moveX += 1

    // Normalize diagonal movement
    if (moveX !== 0 && moveZ !== 0) {
      moveX *= 0.707 // 1/sqrt(2)
      moveZ *= 0.707
    }

    // Movement direction calculated

    // Apply keyboard movement if any keys are pressed
    if (moveX !== 0 || moveZ !== 0) {
      const speed = MOVEMENT_SPEED * (1000 / TARGET_FPS / 1000) // Adjust for frame rate
      const newX = currentPlayer.position.x + moveX * speed
      const newZ = currentPlayer.position.z + moveZ * speed

      // Calculate rotation based on movement direction
      playerRotation = Math.atan2(moveX, moveZ)

      gameStore.update((state) => {
        if (state.currentPlayer) {
          state.currentPlayer.position.set(
            newX,
            0, // Keep player on ground level
            newZ
          )
          isMoving = true
        }
        return state
      })

      // Send position to server periodically
      networkManager.sendPlayerMove({
        x: newX,
        y: 0, // Keep player on ground level
        z: newZ,
      })
    } else {
      isMoving = false
    }
  }

  // Keyboard event handlers
  function handleKeyDown(event: KeyboardEvent) {
    keysPressed.add(event.code)
    event.preventDefault()
  }

  function handleKeyUp(event: KeyboardEvent) {
    keysPressed.delete(event.code)
    event.preventDefault()
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

    // Add keyboard event listeners
    document.addEventListener('keydown', handleKeyDown)
    document.addEventListener('keyup', handleKeyUp)

    return () => {
      stopGameLoop()
      networkManager.disconnect()
      document.removeEventListener('keydown', handleKeyDown)
      document.removeEventListener('keyup', handleKeyUp)
      if (canvas) {
        canvas.removeEventListener('click', handleCanvasClick)
      }
    }
  })

  function handleCanvasClick(event: MouseEvent) {
    if (
      !camera ||
      !groundMesh ||
      !currentPlayer ||
      isMoving ||
      keysPressed.size > 0
    )
      return

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
        y: 0, // Position player on ground level
        z: point.z,
      }

      // Calculate rotation to face target direction
      const dx = clickPosition.x - currentPlayer.position.x
      const dz = clickPosition.z - currentPlayer.position.z
      playerRotation = Math.atan2(dx, dz)

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

<!-- Instanced Grass Meshes -->
{#each grassInstancedMeshes as instancedMesh, index (index)}
  <T is={instancedMesh} />
{/each}

{#if currentPlayer && cameraInitialized}
  <PlayerModel
    position={currentPlayer.position}
    name={currentPlayer.name}
    isCurrentPlayer={true}
    {isMoving}
    rotation={playerRotation}
    cameraPosition={camera?.position}
  />
{/if}

{#if cameraInitialized}
  {#each [...otherPlayers.values()] as player (player.id)}
    <PlayerModel
      position={player.position}
      name={player.name}
      isCurrentPlayer={false}
      cameraPosition={camera?.position}
    />
  {/each}
{/if}

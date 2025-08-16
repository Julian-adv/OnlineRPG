<script lang="ts">
  import { onMount } from 'svelte'
  import { Vector2, Raycaster } from 'three'
  import * as THREE from 'three'
  import { gameStore, type Player } from '../stores/gameStore'
  import { networkManager } from '../network/socket'

  export interface PlayerState {
    state: 'idle' | 'moving'
    speed: number
    direction: number
    position: { x: number; y: number; z: number }
  }

  interface Props {
    onStateChange: (state: PlayerState) => void
    camera?: THREE.PerspectiveCamera
    groundMesh?: THREE.Mesh
  }

  let { onStateChange, camera, groundMesh }: Props = $props()

  let currentPlayer = $state<Player | null>(null)
  let keysPressed = $state(new Set<string>())
  
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

  // Character rotation
  let playerRotation = $state(0)

  // Current player state
  let playerState = $state<PlayerState>({
    state: 'idle',
    speed: 0,
    direction: 0,
    position: { x: 0, y: 0, z: 0 }
  })

  gameStore.subscribe((state) => {
    currentPlayer = state.currentPlayer
    if (currentPlayer) {
      playerState.position = {
        x: currentPlayer.position.x,
        y: currentPlayer.position.y,
        z: currentPlayer.position.z
      }
    }
  })

  // Update player state and notify parent
  function updatePlayerState() {
    const newState: PlayerState = {
      state: isMoving ? 'moving' : 'idle',
      speed: isMoving ? MOVEMENT_SPEED : 0,
      direction: playerRotation,
      position: playerState.position
    }
    
    // Only update if state actually changed
    if (
      newState.state !== playerState.state ||
      newState.speed !== playerState.speed ||
      newState.direction !== playerState.direction
    ) {
      playerState = newState
      onStateChange(newState)
    }
  }

  // Update player movement (click-to-move)
  export function updatePlayerMovement(currentTime: number) {
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

      playerState.position = { x: newX, y: movementTarget.y, z: newZ }
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

      playerState.position = {
        x: movementTarget.x,
        y: movementTarget.y,
        z: movementTarget.z
      }

      // Send final position to server
      networkManager.sendPlayerMove(movementTarget)

      isMoving = false
      movementTarget = null
      movementStartPosition = null
    }

    updatePlayerState()
  }

  // Keyboard movement system
  export function updateKeyboardMovement() {
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

    // Apply keyboard movement if any keys are pressed
    if (moveX !== 0 || moveZ !== 0) {
      const speed = MOVEMENT_SPEED * (1000 / 120 / 1000) // Adjust for frame rate (120 FPS target)
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

      playerState.position = { x: newX, y: 0, z: newZ }

      // Send position to server periodically
      networkManager.sendPlayerMove({
        x: newX,
        y: 0, // Keep player on ground level
        z: newZ
      })
    } else {
      isMoving = false
    }

    updatePlayerState()
  }

  // Handle click-to-move
  export function handleClickToMove(clickPosition: { x: number; y: number; z: number }) {
    if (!currentPlayer || isMoving || keysPressed.size > 0) return

    // Calculate rotation to face target direction
    const dx = clickPosition.x - currentPlayer.position.x
    const dz = clickPosition.z - currentPlayer.position.z
    playerRotation = Math.atan2(dx, dz)

    // Set movement target and start moving
    movementTarget = clickPosition
    movementStartPosition = {
      x: currentPlayer.position.x,
      y: currentPlayer.position.y,
      z: currentPlayer.position.z
    }
    movementStartTime = performance.now()
    isMoving = true

    updatePlayerState()
  }

  // Handle canvas click events
  function handleCanvasClick(event: MouseEvent) {
    if (!camera || !groundMesh || !currentPlayer) return

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

      // Use existing click-to-move logic
      handleClickToMove(clickPosition)
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

  onMount(() => {
    // Add keyboard event listeners
    document.addEventListener('keydown', handleKeyDown)
    document.addEventListener('keyup', handleKeyUp)

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
      document.removeEventListener('keydown', handleKeyDown)
      document.removeEventListener('keyup', handleKeyUp)
      if (canvas) {
        canvas.removeEventListener('mousedown', handleCanvasClick)
      }
    }
  })
</script>
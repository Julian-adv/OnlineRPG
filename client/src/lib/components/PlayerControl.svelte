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
  let movementStartPosition = $state<{
    x: number
    y: number
    z: number
  } | null>(null)
  const MOVEMENT_SPEED = 3 // units per second
  const ACCELERATION = 6 // units per second squared
  const DECELERATION = 6 // units per second squared (same as acceleration for smooth feel)
  
  // Pre-calculate constant distances
  const ACCELERATION_DISTANCE = (MOVEMENT_SPEED * MOVEMENT_SPEED) / (2 * ACCELERATION)
  const DECELERATION_DISTANCE = (MOVEMENT_SPEED * MOVEMENT_SPEED) / (2 * DECELERATION)

  // Character rotation and current speed
  let playerRotation = $state(0)
  let currentSpeed = $state(0) // Current movement speed

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
    const currentPosition = currentPlayer ? {
      x: currentPlayer.position.x,
      y: currentPlayer.position.y,
      z: currentPlayer.position.z
    } : playerState.position

    const newState: PlayerState = {
      state: isMoving ? 'moving' : 'idle',
      speed: currentSpeed, // Use actual current speed instead of fixed MOVEMENT_SPEED
      direction: playerRotation,
      position: currentPosition
    }
    
    // Only update if state actually changed
    if (
      newState.state !== playerState.state ||
      Math.abs(newState.speed - playerState.speed) > 0.01 || // Small tolerance for speed changes
      newState.direction !== playerState.direction ||
      Math.abs(newState.position.x - playerState.position.x) > 0.01 ||
      Math.abs(newState.position.z - playerState.position.z) > 0.01
    ) {
      playerState = newState
      onStateChange(newState)
    }
  }

  // Update player movement (click-to-move) with acceleration/deceleration
  export function updatePlayerMovement(deltaTime: number) {
    if (
      !isMoving ||
      !movementTarget ||
      !currentPlayer ||
      !movementStartPosition
    ) {
      // Reset speed when not moving
      if (currentSpeed > 0) {
        currentSpeed = 0
        updatePlayerState()
      }
      return
    }

    const dx = movementTarget.x - movementStartPosition.x
    const dz = movementTarget.z - movementStartPosition.z
    const totalDistance = Math.sqrt(dx * dx + dz * dz)

    // Calculate current position
    const currentX = currentPlayer.position.x
    const currentZ = currentPlayer.position.z
    const remainingDx = movementTarget.x - currentX
    const remainingDz = movementTarget.z - currentZ
    const remainingDistance = Math.sqrt(remainingDx * remainingDx + remainingDz * remainingDz)

    // Determine which phase we're in and update speed directly
    const traveledDistance = totalDistance - remainingDistance
    const deltaTimeSeconds = deltaTime / 1000 // Convert milliseconds to seconds

    if (traveledDistance < ACCELERATION_DISTANCE) {
      // Acceleration phase - increase speed
      currentSpeed = Math.min(currentSpeed + ACCELERATION * deltaTimeSeconds, MOVEMENT_SPEED)
    } else if (remainingDistance > DECELERATION_DISTANCE) {
      // Cruise phase - maintain max speed
      currentSpeed = MOVEMENT_SPEED
    } else {
      // Deceleration phase - decrease speed
      currentSpeed = Math.max(currentSpeed - DECELERATION * deltaTimeSeconds, 0)
    }

    // Check if we've reached the destination
    if (remainingDistance < 0.01) {
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
      currentSpeed = 0
    } else {
      // Continue movement
      const direction = {
        x: remainingDx / remainingDistance,
        z: remainingDz / remainingDistance
      }

      const moveDistance = currentSpeed * deltaTimeSeconds
      const newX = currentX + direction.x * moveDistance
      const newZ = currentZ + direction.z * moveDistance

      gameStore.update((state) => {
        if (state.currentPlayer) {
          state.currentPlayer.position.set(newX, movementTarget!.y, newZ)
        }
        return state
      })
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
      // Use fixed speed for keyboard movement (instant response)
      currentSpeed = MOVEMENT_SPEED
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

      // Send position to server periodically
      networkManager.sendPlayerMove({
        x: newX,
        y: 0, // Keep player on ground level
        z: newZ
      })
    } else {
      isMoving = false
      currentSpeed = 0
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
<script lang="ts">
  import { T, useLoader } from '@threlte/core'
  import { Text } from '@threlte/extras'
  import type { Vector3 } from 'three'
  import * as THREE from 'three'
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import { onMount } from 'svelte'

  interface Props {
    position: Vector3
    name: string
    isCurrentPlayer: boolean
    isMoving?: boolean
    rotation?: number
    cameraPosition?: Vector3
  }

  let {
    position,
    name,
    isCurrentPlayer,
    isMoving = false,
    rotation = 0,
    cameraPosition,
  }: Props = $props()

  // Calculate nametag rotation to face camera in world space
  function calculateNametagRotation(): [number, number, number] {
    if (!cameraPosition) {
      return [0, 0, 0] // No rotation if no camera
    }

    // Calculate vector from nametag world position to camera
    const nametagWorldX = position.x
    const nametagWorldY = position.y + 2.5 // 2.5 is nametag height
    const nametagWorldZ = position.z

    const dx = cameraPosition.x - nametagWorldX
    const dy = cameraPosition.y - nametagWorldY
    const dz = cameraPosition.z - nametagWorldZ

    // Calculate yaw angle (y rotation) first - horizontal direction to camera
    const yaw = Math.atan2(dx, dz)

    // Calculate horizontal distance for pitch calculation
    const horizontalDistance = Math.sqrt(dx * dx + dz * dz)
    
    // Calculate pitch angle (x rotation) - vertical angle to camera
    const pitch = -Math.atan2(dy, horizontalDistance)

    return [pitch, yaw, 0]
  }

  // GLTF loading
  const gltf = useLoader(GLTFLoader).load('/models/Xbot.glb')

  // Animation system
  let mixer = $state<THREE.AnimationMixer | null>(null)
  let actions = $state<{ [key: string]: THREE.AnimationAction } | null>(null)
  let currentAction = $state<THREE.AnimationAction | null>(null)

  // Animation update loop
  let lastTime = 0
  let animationId: number | null = null
  let lastMovingState = false

  function updateAnimation(time: number) {
    if (mixer) {
      const deltaTime = (time - lastTime) / 1000
      mixer.update(deltaTime)
      lastTime = time

      // Handle movement animation state changes
      if (actions && isMoving !== lastMovingState) {
        const walkAction = actions['walk'] || actions['animation_1']
        const idleAction = actions['idle'] || actions['animation_0']

        if (isMoving && walkAction && currentAction !== walkAction) {
          console.log('Switching to walk animation')
          if (currentAction) {
            currentAction.fadeOut(0.3)
          }
          walkAction.reset().fadeIn(0.3).play()
          currentAction = walkAction
        } else if (!isMoving && idleAction && currentAction !== idleAction) {
          console.log('Switching to idle animation')
          if (currentAction) {
            currentAction.fadeOut(0.3)
          }
          idleAction.reset().fadeIn(0.3).play()
          currentAction = idleAction
        }

        lastMovingState = isMoving
      }
    }
    animationId = requestAnimationFrame(updateAnimation)
  }

  function setupAnimations() {
    if ($gltf && !mixer) {
      console.log('Setting up animations for GLTF model')
      console.log('Available animations:', $gltf.animations.length)

      // Create mixer
      mixer = new THREE.AnimationMixer($gltf.scene)

      // Set up actions (like in Three.js example)
      const animations = $gltf.animations
      actions = {}

      if (animations.length > 0) {
        // Try to find common animation names
        animations.forEach((clip, index) => {
          console.log(`Animation ${index}: ${clip.name}`)
          const action = mixer!.clipAction(clip)

          // Store actions by name or index
          if (clip.name.toLowerCase().includes('idle')) {
            actions!['idle'] = action
          } else if (
            clip.name.toLowerCase().includes('walk') ||
            clip.name.toLowerCase().includes('run')
          ) {
            actions!['walk'] = action
          } else {
            actions![`animation_${index}`] = action
          }
        })

        // If no named animations found, use indices
        if (Object.keys(actions).length === 0) {
          animations.forEach((clip, index) => {
            actions![`animation_${index}`] = mixer!.clipAction(clip)
          })
        }

        console.log('Available actions:', Object.keys(actions))

        // Start with idle or first available animation
        const idleAction = actions['idle'] || actions['animation_0']
        if (idleAction) {
          idleAction.play()
          currentAction = idleAction
        }

        // Start animation loop
        lastTime = performance.now()
        animationId = requestAnimationFrame(updateAnimation)
      }
    }
  }

  onMount(() => {
    // Wait for GLTF to load and setup animations
    const checkGltf = () => {
      if ($gltf) {
        setupAnimations()
      } else {
        setTimeout(checkGltf, 100)
      }
    }
    checkGltf()

    // Cleanup on unmount
    return () => {
      if (animationId) {
        cancelAnimationFrame(animationId)
      }
    }
  })
</script>

<!-- Character Model -->
<T.Group
  position={[position.x, position.y, position.z]}
  rotation={[0, rotation, 0]}
>
  <!-- 3D Character Model -->
  {#if $gltf}
    <T is={$gltf.scene} />
  {/if}
</T.Group>

<!-- Name tag (separate from character to avoid rotation inheritance) -->
<Text
  text={name}
  position={[position.x, position.y + 2.5, position.z]}
  rotation={calculateNametagRotation()}
  fontSize={0.3}
  color={isCurrentPlayer ? '#4299e1' : '#ffffff'}
  anchorX="center"
  anchorY="middle"
/>

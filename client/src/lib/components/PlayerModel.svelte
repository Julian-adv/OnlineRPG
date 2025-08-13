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
      return [0, 0, 0]
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

  // Load static model instead of animated one
  const gltf = useLoader(GLTFLoader).load(
    '/models/static_1_Armature011_66.glb'
  )

  // Simple animation system
  let simpleAnimation = $state<{
    update: (deltaTime: number) => void
    isPlaying: boolean
    startTime: number
  } | null>(null)
  let animationId: number | null = null
  let lastTime = 0
  // No need for yOffset anymore - models are pre-positioned with feet at origin

  function updateAnimation(time: number) {
    const deltaTime = (time - lastTime) / 1000
    lastTime = time

    if (simpleAnimation && $gltf) {
      simpleAnimation.update(deltaTime)
    }

    animationId = requestAnimationFrame(updateAnimation)
  }

  function setupSimpleAnimation() {
    if ($gltf && !simpleAnimation) {
      console.log('Setting up simple animation for static model')

      console.log('Model should already be positioned with feet at origin')

      // Enable shadows on all meshes in the model
      $gltf.scene.traverse((child) => {
        if (child instanceof THREE.Mesh) {
          child.castShadow = true
          child.receiveShadow = true
        }
      })

      // Create simple animation system
      simpleAnimation = {
        isPlaying: true,
        startTime: performance.now(),
        update(_deltaTime: number) {
          if (!$gltf || !this.isPlaying) return

          const elapsed = (performance.now() - this.startTime) / 1000
          const model = $gltf.scene

          if (isMoving) {
            // Walking animation: slight bounce and forward lean
            model.position.y = Math.sin(elapsed * 8) * 0.05 // Faster bounce when walking
            model.rotation.x = Math.sin(elapsed * 8) * 0.02 // Slight forward lean rhythm
            
            // Slight side-to-side sway
            model.rotation.z = Math.sin(elapsed * 6) * 0.01
          } else {
            // Idle animation: gentle breathing and subtle movement
            model.position.y = Math.sin(elapsed * 2) * 0.02 // Gentle breathing
            model.rotation.x = Math.sin(elapsed * 1.5) * 0.005 // Very subtle head movement
            
            // Occasional slight turn
            model.rotation.y = Math.sin(elapsed * 0.8) * 0.01
          }
        }
      }

      // Start animation loop
      lastTime = performance.now()
      animationId = requestAnimationFrame(updateAnimation)
      
      console.log('Simple animation system initialized')
    }
  }

  onMount(() => {
    // Wait for GLTF to load and setup simple animations
    const checkGltf = () => {
      if ($gltf) {
        setupSimpleAnimation()
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
      if (simpleAnimation) {
        simpleAnimation.isPlaying = false
      }
    }
  })
</script>

<!-- Character Model -->
<T.Group
  position={[position.x, position.y, position.z]}
  rotation={[0, rotation, 0]}
>
  <!-- 3D Character Model (pre-positioned with feet at origin) -->
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

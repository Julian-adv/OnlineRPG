<script lang="ts">
  import { T, useLoader } from '@threlte/core'
  import { Text } from '@threlte/extras'
  import type { Vector3 } from 'three'
  import * as THREE from 'three'
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import * as SkeletonUtils from 'three/examples/jsm/utils/SkeletonUtils.js'
  import { onMount } from 'svelte'
  import { SvelteSet } from 'svelte/reactivity'

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
    isMoving,
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

  // Load animated model
  const gltf = useLoader(GLTFLoader).load('/models/merged (3).glb')

  // Animation system - following gpt-all-in-one.html approach
  let mixer: THREE.AnimationMixer | null = null
  let currentAction: THREE.AnimationAction | null = null
  let animationId: number | null = null
  let modelRoot = $state<THREE.Group | null>(null) // ✅ $state로 반응성 추가
  let clock = new THREE.Clock()

  function updateAnimation() {
    const deltaTime = clock.getDelta()

    if (mixer) {
      mixer.update(deltaTime)
    }

    animationId = requestAnimationFrame(updateAnimation)
  }

  let validAnimations: THREE.AnimationClip[] = []
  let lastMovingState: boolean | undefined = undefined

  const playAnimationForState = () => {
    if (!mixer || validAnimations.length === 0) return

    // Select animation based on movement state
    let clip: THREE.AnimationClip
    if (isMoving === false) {
      // Find Animation_2 or fallback to index 2
      clip =
        validAnimations.find((anim) => anim.name === 'Animation_2') ||
        validAnimations[2]
      console.log(`Playing idle animation: ${clip.name}`)
    } else {
      // Use default animation (index 2) for movement
      clip = validAnimations[0]
      console.log(`Playing movement animation: ${clip.name}`)
    }

    const newAction = mixer.clipAction(clip)
    
    // Setup new action
    newAction.reset()
    newAction.loop = THREE.LoopRepeat
    newAction.paused = false
    
    // If there's a current action, crossfade to the new one
    if (currentAction && currentAction !== newAction) {
      const crossfadeDuration = 0.3 // 300ms crossfade
      
      // Use THREE.js built-in crossfade
      newAction.crossFadeFrom(currentAction, crossfadeDuration, true)
    }
    
    // Play the new action
    newAction.play()
    currentAction = newAction
  }

  function setupRealAnimation() {
    if ($gltf && !mixer && !modelRoot) {
      console.log('Setting up real animation system')

      // Create a safely cloned model using SkeletonUtils - gpt-all-in-one.html 패턴 따름
      const cloned = SkeletonUtils.clone($gltf.scene)
      const newModelRoot = new THREE.Group()
      newModelRoot.add(cloned)

      // Enable shadows on all meshes
      newModelRoot.traverse((child) => {
        if (child instanceof THREE.Mesh) {
          child.castShadow = true
          child.receiveShadow = true
        }
      })

      // Filter animations to only include tracks that match model nodes
      const animations = $gltf.animations || []
      console.log(`Found ${animations.length} animation clips`)

      // Collect all node names in the cloned model
      const modelNodeNames = new SvelteSet()
      cloned.traverse((obj) => {
        if (obj.name) modelNodeNames.add(obj.name)
      })
      console.log(`Model has ${modelNodeNames.size} named nodes`)
      console.log('Model node names:', Array.from(modelNodeNames).slice(0, 10))

      // Filter animations to only include tracks that target existing nodes
      validAnimations = animations.filter((clip) => {
        console.log(
          `Checking clip: ${clip.name} with ${clip.tracks.length} tracks`
        )

        const validTracks = clip.tracks.filter((track) => {
          const targetName = track.name.split('.')[0]
          const isValid = modelNodeNames.has(targetName)
          if (!isValid) {
            console.log(
              `  ❌ Track "${track.name}" targets "${targetName}" (not found in model)`
            )
          }
          return isValid
        })

        console.log(
          `  ✅ Clip "${clip.name}": ${validTracks.length}/${clip.tracks.length} tracks valid`
        )
        return validTracks.length > 0
      })

      console.log(`Found ${validAnimations.length} valid animations`)

      if (validAnimations.length > 0) {
        // Setup mixer
        mixer = new THREE.AnimationMixer(newModelRoot)

        // Play appropriate animation based on isMoving state
        playAnimationForState()

        // Start animation loop
        clock.start()
        animationId = requestAnimationFrame(updateAnimation)
      } else {
        console.warn('No suitable animations found with strict filtering')

        // Fallback: try to play any animation without filtering
        if (animations.length > 0) {
          console.log(
            'Trying fallback: playing first animation without filtering'
          )
          mixer = new THREE.AnimationMixer(newModelRoot)
          const clip = animations[0]
          console.log(
            `Playing fallback animation: ${clip.name}, duration: ${clip.duration}s`
          )

          currentAction = mixer.clipAction(clip)
          currentAction.reset()
          currentAction.loop = THREE.LoopRepeat
          currentAction.paused = false
          currentAction.play()

          // Start animation loop
          clock.start()
          animationId = requestAnimationFrame(updateAnimation)
        } else {
          console.log('No animations available at all')
        }
      }

      modelRoot = newModelRoot
    }
  }

  onMount(() => {
    // Wait for GLTF to load and setup real animation
    const checkGltf = () => {
      if ($gltf) {
        setupRealAnimation()
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
      if (mixer) {
        mixer.stopAllAction()
        mixer = null
      }
      if (modelRoot) {
        modelRoot = null
      }
    }
  })

  // Function to update animation state - called from GameScene gameLoop
  export function updateAnimationState() {
    if (mixer && validAnimations.length > 0) {
      // Only update animation if the movement state has changed
      if (lastMovingState !== isMoving) {
        lastMovingState = isMoving
        playAnimationForState()
      }
    }
  }
</script>

<!-- Character Model -->
{#if modelRoot}
  <T.Group
    position={[position.x, position.y, position.z]}
    rotation={[0, rotation, 0]}
  >
    <!-- 3D Character Model with real animations -->
    <T is={modelRoot} />
  </T.Group>
{/if}

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

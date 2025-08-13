<script lang="ts">
  import { T, useLoader } from '@threlte/core'
  import { Text } from '@threlte/extras'
  import type { Vector3 } from 'three'
  import * as THREE from 'three'
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import * as SkeletonUtils from 'three/examples/jsm/utils/SkeletonUtils.js'
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

  // Load animated model
  const gltf = useLoader(GLTFLoader).load(
    '/models/animated_13_Armature006_870.glb'
  )

  // Animation system - following gpt-all-in-one.html approach
  let mixer: THREE.AnimationMixer | null = null
  let currentAction: THREE.AnimationAction | null = null
  let animationId: number | null = null
  let lastMovingState = false
  let modelRoot = $state<THREE.Group | null>(null) // ✅ $state로 반응성 추가
  let clock = new THREE.Clock()

  function updateAnimation() {
    const deltaTime = clock.getDelta()

    if (mixer) {
      mixer.update(deltaTime)

      // Handle movement state changes
      if (isMoving !== lastMovingState) {
        if (currentAction) {
          if (isMoving) {
            currentAction.timeScale = 2.0 // Faster when moving
          } else {
            currentAction.timeScale = 1.0 // Normal speed when idle
          }
        }
        lastMovingState = isMoving
      }
    }

    animationId = requestAnimationFrame(updateAnimation)
  }

  function collectNodeNames(root: THREE.Object3D): Set<string> {
    const set = new Set<string>()
    root.traverse((obj) => {
      if (obj.name) set.add(obj.name)
    })
    return set
  }

  function filterAnimations(
    anims: THREE.AnimationClip[],
    allowedNames: Set<string>
  ): THREE.AnimationClip[] {
    const out: THREE.AnimationClip[] = []
    for (const clip of anims) {
      const kept = clip.tracks.filter((track) => {
        // track name: "NodeName.property" format
        const target = track.name.split('.')[0]
        return allowedNames.has(target)
      })
      if (kept.length > 0) {
        const newClip = clip.clone()
        newClip.tracks = kept
        out.push(newClip)
      }
    }
    return out
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

      // Collect node names for animation filtering
      const allowedNames = collectNodeNames(cloned)
      console.log(`Found ${allowedNames.size} node names for filtering`)

      // Filter animations based on node names
      const animations = $gltf.animations || []
      const relatedClips = filterAnimations(animations, allowedNames)
      console.log(`Found ${relatedClips.length} relevant animation clips`)

      if (relatedClips.length > 0) {
        // Setup mixer and play first animation - gpt-all-in-one.html 패턴
        mixer = new THREE.AnimationMixer(newModelRoot)
        const clip = relatedClips[0]
        console.log(
          `Playing animation: ${clip.name}, duration: ${clip.duration}s`
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
        console.warn('No suitable animations found')
      }

      // ✅ 반응성 있는 상태로 설정
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

<script lang="ts">
  import { T, useTask, useThrelte } from '@threlte/core'
  import * as THREE from 'three'
  import { PMREMGenerator, type WebGPURenderer } from 'three/webgpu'
  import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js'
  import { SvelteMap } from 'svelte/reactivity'
  import {
    computeSoleGroundOffset,
    createCharacterModelRoot,
    getGltfAnimations,
  } from '../utils/characterAnimationUtils'
  import {
    CHARACTER_ANIMATION_PACK_PATHS,
    getCharacterModelPath,
  } from '../utils/modelPaths'
  import {
    getEffectivePreset,
    graphicsQuality,
  } from '../stores/graphicsSettings'
  import { createRenderCadence } from '../utils/renderCadence'
  import { loadGLB } from '../utils/gltfCache'
  import type { CharacterClass, Gender } from '../network/networkTypes'

  interface Props {
    /** Social-pack clip to loop, or null to show nothing. */
    anim: string | null
    characterClass: CharacterClass
    gender: Gender
    /** True once a clip is actually posing the model, so the wrapper can
     *  stay hidden through the load window instead of framing an empty box. */
    playing?: boolean
  }

  let {
    anim,
    characterClass,
    gender,
    playing = $bindable(false),
  }: Props = $props()

  const { renderer: _renderer, scene, invalidate } = useThrelte()
  const renderer = _renderer as unknown as WebGPURenderer

  const CAMERA_FOV = 32
  const CAMERA_POSITION: [number, number, number] = [0, 1.05, 3.4]
  const CAMERA_LOOK_AT_Y = 0.9

  $effect(() => {
    let envRt: THREE.RenderTarget | null = null
    let disposed = false
    renderer.init().then(() => {
      if (disposed) return
      const pmremGenerator = new PMREMGenerator(renderer)
      const room = new RoomEnvironment()
      envRt = pmremGenerator.fromScene(room)
      scene.environment = envRt.texture
      scene.environmentIntensity = 0.55
      room.traverse((o) => (o as THREE.Mesh).geometry?.dispose())
      pmremGenerator.dispose()
      invalidate()
    })
    return () => {
      disposed = true
      scene.environment = null
      envRt?.dispose()
    }
  })

  let modelRoot = $state<THREE.Group | null>(null)
  let mixer = $state<THREE.AnimationMixer | null>(null)
  let currentAction: THREE.AnimationAction | null = null
  const clipsByName = new SvelteMap<string, THREE.AnimationClip>()

  const modelPath = $derived(getCharacterModelPath(characterClass, gender))

  $effect(() => {
    const path = modelPath
    let cancelled = false
    Promise.all([
      loadGLB(path),
      loadGLB(CHARACTER_ANIMATION_PACK_PATHS.social),
    ]).then(([charGltf, socialGltf]) => {
      if (cancelled) return
      for (const clip of getGltfAnimations(socialGltf)) {
        clipsByName.set(clip.name, clip)
      }
      const { modelRoot: root } = createCharacterModelRoot(charGltf.scene)
      root.position.y = computeSoleGroundOffset(root)
      mixer = new THREE.AnimationMixer(root)
      modelRoot = root
      // One hidden bind-pose frame so the character's pipelines compile
      // before the wrapper ever becomes visible.
      invalidate()
    })
    return () => {
      cancelled = true
      mixer?.stopAllAction()
      mixer = null
      currentAction = null
      modelRoot = null
      playing = false
    }
  })

  $effect(() => {
    const name = anim
    if (!name || !mixer || !modelRoot) return
    const clip = clipsByName.get(name)
    if (!clip) return

    const action = mixer.clipAction(clip)
    // Re-hovering the same clip resumes it; only a new clip restarts. Hard
    // switch, no crossfade — interrupted three.js fades snap to stale weights.
    if (currentAction !== action) {
      currentAction?.stop()
      action.reset()
      action.loop = THREE.LoopRepeat
      action.play()
    }
    // Pose immediately so the first visible frame is mid-clip, not bind pose.
    mixer.update(0)
    currentAction = action
    playing = true
    invalidate()
  })

  // Manual invalidation: nothing hovered means no renders at all. Steps at the
  // preset's frame cap so the preview can't out-render the main scene.
  const cadence = $derived(
    createRenderCadence(getEffectivePreset($graphicsQuality).maxRenderFps)
  )
  let pendingDelta = 0
  useTask(
    (delta) => {
      if (!mixer || !currentAction || !anim) return
      pendingDelta += delta
      if (!cadence.shouldRender(delta * 1000)) return
      mixer.update(Math.min(pendingDelta, 0.1))
      pendingDelta = 0
      invalidate()
    },
    { autoInvalidate: false }
  )
</script>

<T.PerspectiveCamera
  makeDefault
  fov={CAMERA_FOV}
  position={CAMERA_POSITION}
  oncreate={(cam) => cam.lookAt(0, CAMERA_LOOK_AT_Y, 0)}
/>

<T.AmbientLight intensity={0.35} />
<T.DirectionalLight position={[1.5, 3, 2.5]} intensity={1.4} />

{#if modelRoot}
  <T is={modelRoot} />
{/if}

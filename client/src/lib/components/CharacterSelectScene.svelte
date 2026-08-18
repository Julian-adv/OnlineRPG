<script lang="ts">
  import { T, useThrelte, useTask } from '@threlte/core'
  import * as THREE from 'three'
  import { PMREMGenerator, type WebGPURenderer } from 'three/webgpu'
  import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js'
  import { onMount } from 'svelte'
  import { interactivity, type IntersectionEvent } from '@threlte/extras'
  import type { AccountCharacter } from '../network/socket'
  import CharacterPreview from './CharacterPreview.svelte'
  import CharacterSlotLabel from './CharacterSlotLabel.svelte'
  import { loadSplatLayers } from '../utils/splatLayerLoader'
  import { loadGLB } from '../utils/gltfCache'
  import { getWeaponModelPath } from '../utils/modelPaths'

  /** Pointer travel that makes a gesture a drag, so spinning never selects.
   *  Threlte enforces it on click and dblclick; only the press *before* a
   *  double is ours to check. */
  const DRAG_CLICK_SLOP_PX = 5

  interactivity({ clickDistanceThreshold: DRAG_CLICK_SLOP_PX })

  // Preload assets needed by game scene so they're cached when it mounts
  loadSplatLayers()
  for (const model of ['weapons/sword.glb', 'weapons/spear.glb']) {
    loadGLB(getWeaponModelPath(model))
  }

  interface Props {
    characters: AccountCharacter[]
    selectedCharacterId: number | null
    onSlotClick: (slotIndex: number) => void
    onSlotDoubleClick: (slotIndex: number) => void
  }

  let {
    characters,
    selectedCharacterId,
    onSlotClick,
    onSlotDoubleClick,
  }: Props = $props()

  const SLOT_SPACING = 1.8
  const SLOT_POSITIONS = [-SLOT_SPACING, 0, SLOT_SPACING]
  const SLOT_DEPTH = 2.5
  const SLOT_DISC_RADIUS = 0.76
  const SLOT_DISC_THICKNESS = 0.1
  const SLOT_DISC_Y = SLOT_DISC_THICKNESS / 2 + 0.002
  const SLOT_HITBOX_WIDTH = 1.35
  const SLOT_HITBOX_HEIGHT = 2.5
  const SLOT_HITBOX_DEPTH = 1.25
  const CHARACTER_Y_OFFSET = SLOT_DISC_THICKNESS
  const PLATFORM_MARGIN_X = 2.8
  const PLATFORM_MARGIN_Z_FRONT = 3.2
  const PLATFORM_MARGIN_Z_BACK = 4.2
  const PLATFORM_SCALE = 4
  const BASE_PLATFORM_WIDTH = SLOT_SPACING * 2 + PLATFORM_MARGIN_X * 2
  const BASE_PLATFORM_DEPTH =
    PLATFORM_MARGIN_Z_FRONT + PLATFORM_MARGIN_Z_BACK + SLOT_DEPTH
  const PLATFORM_WIDTH = BASE_PLATFORM_WIDTH * PLATFORM_SCALE
  const PLATFORM_DEPTH = BASE_PLATFORM_DEPTH * PLATFORM_SCALE
  const PLATFORM_CENTER_Z =
    (SLOT_DEPTH + PLATFORM_MARGIN_Z_FRONT - PLATFORM_MARGIN_Z_BACK) / 2
  const CAMERA_FOV = 45
  const CAMERA_POSITION_Y = 1.5
  const CAMERA_LOOK_AT_Y = 0.8
  const CHARACTER_HALF_WIDTH = 1.0
  const CHARACTER_HALF_HEIGHT = 1.8
  const CAMERA_FIT_PADDING = 1.1
  const MAX_CAMERA_WIDTH = 1280
  const AMBIENT_INTENSITY = 0.12
  const KEY_LIGHT_INTENSITY = 0.05
  const FILL_LIGHT_INTENSITY = 0.48
  /** Same feel as the create screen's drag-to-spin. */
  const DRAG_RADIANS_PER_PIXEL = 0.01

  const { size, renderer: _renderer, scene } = useThrelte()
  // Cast renderer — Threlte types it as WebGLRenderer but we use WebGPURenderer via createRenderer
  const renderer = _renderer as unknown as WebGPURenderer
  let viewportSize = $state({ width: 1, height: 1 })
  let useCompactSlotLabels = $derived(
    viewportSize.width <= 600 || viewportSize.height <= 700
  )
  let cameraPositionZ = $state(8)

  let cameraRef = $state<THREE.PerspectiveCamera | undefined>(undefined)

  interface CharacterPreviewInstance {
    isGltfReady(): boolean
    isSetUp(): boolean
    setup(): void
    update(delta: number): void
    dispose(): void
  }

  // CharacterPreview refs managed by bind:this
  let characterPreviews: (CharacterPreviewInstance | undefined)[] = $state([
    undefined,
    undefined,
    undefined,
  ])
  // Single pair of spotlights — created once via Three.js, moved in useTask.
  // Avoids WebGPU pipeline recompilation on selection changes.
  const spotlightTarget = new THREE.Object3D()
  const keyLight = new THREE.SpotLight('#ffffff', 9.0, 14, 0.34, 0.22, 1.2)
  keyLight.castShadow = true
  keyLight.shadow.mapSize.set(2048, 2048)
  keyLight.shadow.camera.near = 0.5
  keyLight.shadow.camera.far = 18
  keyLight.shadow.bias = -0.0002
  keyLight.shadow.normalBias = 0.02
  keyLight.target = spotlightTarget

  const fillLight = new THREE.SpotLight('#fff2d8', 3.4, 12, 0.52, 0.8, 1.2)
  fillLight.target = spotlightTarget

  let spotlightsAdded = false

  // Drag-to-rotate, one angle per slot so a spun character keeps its pose
  // while you look at another.
  let slotRotations = $state([0, 0, 0])
  let dragSlot: number | null = null
  let dragLastX = 0
  let dragTravel = 0
  let previousDragTravel = 0

  function startSlotDrag(
    slotIndex: number,
    event: IntersectionEvent<PointerEvent>
  ) {
    // Left button only: a right-press whose release the context menu swallows
    // would leave the slot stuck to the pointer.
    if (event.nativeEvent.button !== 0 || !characters[slotIndex]) return
    dragSlot = slotIndex
    // The floor lies behind every slot, so without this its handler would
    // fire too and hand the drag to the selected character instead.
    event.stopPropagation()
  }

  /** Dragging the floor spins whoever is selected — the character hitboxes are
   *  narrow, and there is nothing else to grab out there. */
  function startSelectedDrag(event: IntersectionEvent<PointerEvent>) {
    startSlotDrag(getSelectedSlotIndex(), event)
  }

  function handleWindowPointerDown(event: PointerEvent) {
    previousDragTravel = dragTravel
    dragLastX = event.clientX
    dragTravel = 0
  }

  function handleWindowPointerMove(event: PointerEvent) {
    if (dragSlot === null) return
    const dx = event.clientX - dragLastX
    dragLastX = event.clientX
    dragTravel += Math.abs(dx)
    slotRotations[dragSlot] += dx * DRAG_RADIANS_PER_PIXEL
  }

  function endSlotDrag() {
    dragSlot = null
  }

  /** The browser still counts a drag's release as the first click of a double,
   *  and Threlte only measures the second press — so entering the game has to
   *  check the first, or "rotate, then click to select" launches the
   *  character. */
  function precededByDrag(): boolean {
    return previousDragTravel > DRAG_CLICK_SLOP_PX
  }

  function getSelectedSlotIndex(): number {
    if (selectedCharacterId === null) return -1
    return characters.findIndex((c) => c.id === selectedCharacterId)
  }

  function getSelectedSlotX(): number | null {
    const idx = getSelectedSlotIndex()
    return idx >= 0 ? SLOT_POSITIONS[idx] : null
  }

  onMount(() => {
    const unsubscribe = size.subscribe((nextSize) => {
      viewportSize = nextSize
    })

    // On window, not the canvas: a spin that runs off the character (or off
    // the canvas entirely) should keep turning until the button comes up.
    window.addEventListener('pointerdown', handleWindowPointerDown)
    window.addEventListener('pointermove', handleWindowPointerMove)
    window.addEventListener('pointerup', endSlotDrag)
    window.addEventListener('pointercancel', endSlotDrag)

    // Set scene background to match the character select gradient
    scene.background = new THREE.Color('#1a2a40')

    renderer.init().then(() => {
      const pmremGenerator = new PMREMGenerator(renderer)
      const rt = pmremGenerator.fromScene(new RoomEnvironment())
      scene.environment = rt.texture
      scene.environmentIntensity = 0.1
      pmremGenerator.dispose()

      // Pre-compile all WebGPU shaders (characters, platform, lights)
      requestAnimationFrame(() => {
        if (cameraRef) {
          renderer.compileAsync(scene, cameraRef).catch(() => {})
        }
      })
    })

    return () => {
      window.removeEventListener('pointerdown', handleWindowPointerDown)
      window.removeEventListener('pointermove', handleWindowPointerMove)
      window.removeEventListener('pointerup', endSlotDrag)
      window.removeEventListener('pointercancel', endSlotDrag)
      scene.background = null
      scene.environment?.dispose()
      scene.environment = null
      unsubscribe()
      // Clean up spotlights
      if (spotlightsAdded) {
        scene.remove(keyLight, fillLight, spotlightTarget)
        spotlightsAdded = false
      }
    }
  })

  function calculateCameraPositionZ(width: number, height: number) {
    const safeWidth = Math.min(Math.max(1, width), MAX_CAMERA_WIDTH)
    const safeHeight = Math.max(1, height)
    const aspect = safeWidth / safeHeight

    const halfVerticalFov = THREE.MathUtils.degToRad(CAMERA_FOV / 2)
    const halfHorizontalFov = Math.atan(Math.tan(halfVerticalFov) * aspect)

    const halfSpanX = SLOT_SPACING + CHARACTER_HALF_WIDTH
    const fitDistanceByWidth = halfSpanX / Math.tan(halfHorizontalFov)
    const fitDistanceByHeight =
      CHARACTER_HALF_HEIGHT / Math.tan(halfVerticalFov)
    const offsetZ =
      Math.max(fitDistanceByWidth, fitDistanceByHeight) * CAMERA_FIT_PADDING

    return SLOT_DEPTH + offsetZ
  }

  $effect(() => {
    cameraPositionZ = calculateCameraPositionZ(
      viewportSize.width,
      viewportSize.height
    )

    if (cameraRef) {
      cameraRef.lookAt(0, CAMERA_LOOK_AT_Y, SLOT_DEPTH)
    }
  })

  // Central game loop — staggered setup + per-frame update + spotlight positioning
  useTask((delta) => {
    // Phase 1: Staggered setup — one character per frame
    for (const preview of characterPreviews) {
      if (preview && preview.isGltfReady() && !preview.isSetUp()) {
        preview.setup()
        if (!spotlightsAdded) {
          scene.add(spotlightTarget, keyLight, fillLight)
          spotlightsAdded = true
        }
        break // one per frame
      }
    }

    // Phase 2: Move spotlights to selected character (pure Three.js mutation — no pipeline change)
    const slotX = getSelectedSlotX()
    if (slotX !== null && spotlightsAdded) {
      const y = CHARACTER_Y_OFFSET
      const z = SLOT_DEPTH
      spotlightTarget.position.set(slotX, y + 0.9, z)
      keyLight.position.set(slotX, y + 4.0, z + 1.2)
      fillLight.position.set(slotX, y + 2.5, z + 3.1)
    }

    // Phase 3: Update all set-up characters
    for (const preview of characterPreviews) {
      if (preview?.isSetUp()) {
        preview.update(delta)
      }
    }
  })
</script>

<T.PerspectiveCamera
  makeDefault
  position={[0, CAMERA_POSITION_Y, cameraPositionZ]}
  fov={CAMERA_FOV}
  bind:ref={cameraRef}
/>

<T.AmbientLight intensity={AMBIENT_INTENSITY} />
<T.DirectionalLight
  position={[5, 8, 5]}
  intensity={KEY_LIGHT_INTENSITY}
  castShadow
  shadow.camera.left={-8}
  shadow.camera.right={8}
  shadow.camera.top={8}
  shadow.camera.bottom={-8}
  shadow.camera.near={0.5}
  shadow.camera.far={24}
  shadow.mapSize.width={1024}
  shadow.mapSize.height={1024}
  shadow.bias={-0.00025}
  shadow.normalBias={0.02}
/>
<T.DirectionalLight
  position={[-3, 6, -2]}
  intensity={FILL_LIGHT_INTENSITY}
  color="#8899cc"
/>

<T.Mesh
  rotation.x={-Math.PI / 2}
  position={[0, -0.01, PLATFORM_CENTER_Z]}
  receiveShadow
  onpointerdown={startSelectedDrag}
>
  <T.PlaneGeometry args={[PLATFORM_WIDTH, PLATFORM_DEPTH]} />
  <T.MeshStandardMaterial
    color="#1a2535"
    opacity={0.6}
    transparent
    depthWrite={false}
    envMapIntensity={0}
  />
</T.Mesh>

{#each [0, 1, 2] as slotIndex (slotIndex)}
  {@const character = characters[slotIndex]}
  <T.Mesh
    position={[SLOT_POSITIONS[slotIndex], SLOT_DISC_Y, SLOT_DEPTH]}
    receiveShadow
    onpointerdown={(e: IntersectionEvent<PointerEvent>) =>
      startSlotDrag(slotIndex, e)}
    onclick={() => onSlotClick(slotIndex)}
    ondblclick={() => !precededByDrag() && onSlotDoubleClick(slotIndex)}
  >
    <T.CylinderGeometry
      args={[SLOT_DISC_RADIUS, SLOT_DISC_RADIUS, SLOT_DISC_THICKNESS, 40]}
    />
    <T.MeshStandardMaterial
      color="#2f3f52"
      opacity={1.0}
      transparent
      envMapIntensity={0}
    />
  </T.Mesh>

  <T.Mesh
    position={[SLOT_POSITIONS[slotIndex], SLOT_HITBOX_HEIGHT / 2, SLOT_DEPTH]}
    onpointerdown={(e: IntersectionEvent<PointerEvent>) =>
      startSlotDrag(slotIndex, e)}
    onclick={() => onSlotClick(slotIndex)}
    ondblclick={() => !precededByDrag() && onSlotDoubleClick(slotIndex)}
  >
    <T.BoxGeometry
      args={[SLOT_HITBOX_WIDTH, SLOT_HITBOX_HEIGHT, SLOT_HITBOX_DEPTH]}
    />
    <T.MeshBasicMaterial
      color="#ffffff"
      opacity={0}
      transparent
      depthWrite={false}
    />
  </T.Mesh>

  {#if character}
    {#key character.id}
      <CharacterPreview
        bind:this={characterPreviews[slotIndex]}
        positionX={SLOT_POSITIONS[slotIndex]}
        positionY={CHARACTER_Y_OFFSET}
        positionZ={SLOT_DEPTH}
        rotationY={slotRotations[slotIndex]}
        selected={character.id === selectedCharacterId}
        characterClass={character.class}
        gender={character.gender}
        equipment={character.equipment}
        camera={cameraRef}
      />
    {/key}
  {/if}

  <CharacterSlotLabel
    {character}
    selected={character?.id === selectedCharacterId}
    positionX={SLOT_POSITIONS[slotIndex]}
    positionZ={SLOT_DEPTH}
    camera={cameraRef}
    onclick={() => onSlotClick(slotIndex)}
    ondblclick={() => !precededByDrag() && onSlotDoubleClick(slotIndex)}
    compact={useCompactSlotLabels}
  />
{/each}

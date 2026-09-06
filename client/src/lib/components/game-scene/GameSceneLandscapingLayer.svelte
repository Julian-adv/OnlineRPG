<script lang="ts">
  import { T, useTask, useThrelte } from '@threlte/core'
  import { onMount } from 'svelte'
  import { get } from 'svelte/store'
  import * as THREE from 'three'
  import {
    landscapingMode,
    landscapingPending,
    landscapingError,
    landscapingHint,
    landscapingRoadStart,
    hasLandscapingToolbox,
  } from '../../stores/landscapingStore'
  import {
    brushSize,
    brushStrength,
    splatLayer,
  } from '../../stores/editorStore'
  import { cameraRotationEnabled } from '../../stores/debugStore'
  import { networkManager } from '../../network/socket'
  import {
    landscapingSamples,
    ownsEstatePosition,
    snapBrushCoordinate,
    type LandscapingStroke,
  } from '../../terrain/landscaping'
  import { wrapWorldX, unwrapWorldXNear } from '../../terrain/world-wrap'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'
  import type { LocalPlayer } from '../../stores/gameStore'
  import { isAdminUser } from '../../stores/gameStore'
  import { LandscapingStrokes } from '../../terrain/landscaping-strokes'

  let {
    heightManager,
    terrainMeshes,
    player,
  }: {
    heightManager: TerrainHeightManager
    terrainMeshes: (THREE.Mesh | undefined)[]
    player: LocalPlayer | null
  } = $props()
  const { camera, renderer } = useThrelte()
  const raycaster = new THREE.Raycaster()
  const pointer = new THREE.Vector2()
  const material = new THREE.MeshBasicMaterial({
    color: '#83dba1',
    transparent: true,
    opacity: 0.3,
    depthWrite: false,
    side: THREE.DoubleSide,
  })
  const preview = new THREE.Mesh(new THREE.BufferGeometry(), material)
  preview.renderOrder = 11
  preview.visible = false
  let cursor: { x: number; y: number } | null = null
  let painting = false
  let elapsed = 0
  let previewKey = ''
  let currentStroke: LandscapingStroke | null = null
  let canPaint = false
  const strokes = new LandscapingStrokes()

  function updatePreview() {
    const mode = get(landscapingMode)
    if (
      !mode ||
      mode.tool === 'Fence' ||
      !cursor ||
      !player ||
      get(cameraRotationEnabled)
    ) {
      preview.visible = false
      currentStroke = null
      previewKey = ''
      return
    }
    const rect = renderer.domElement.getBoundingClientRect()
    pointer.set(
      ((cursor.x - rect.left) / rect.width) * 2 - 1,
      -((cursor.y - rect.top) / rect.height) * 2 + 1
    )
    raycaster.setFromCamera(pointer, get(camera))
    const hit = raycaster.intersectObjects(
      terrainMeshes.filter((m): m is THREE.Mesh => !!m),
      false
    )[0]
    if (!hit) {
      preview.visible = false
      currentStroke = null
      previewKey = ''
      return
    }
    const radius = get(brushSize)
    const point: [number, number] = [
      wrapWorldX(snapBrushCoordinate(hit.point.x, radius)),
      snapBrushCoordinate(hit.point.z, radius),
    ]
    const start = mode.tool === 'Road' ? get(landscapingRoadStart) : null
    currentStroke = {
      start: start ?? point,
      end: start ? point : null,
      radius,
      strength: get(brushStrength),
      palette: get(splatLayer),
    }
    const isAdmin = get(isAdminUser)
    const reason = !get(hasLandscapingToolbox)
      ? "Carry a Landscaper's Toolbox to paint"
      : !isAdmin &&
          !ownsEstatePosition(mode.plots, player.position.x, player.position.z)
        ? 'Stand inside your estate to paint'
        : !mode.palette.includes(currentStroke.palette)
          ? 'Learn this material before painting'
          : null
    const key = JSON.stringify([
      currentStroke,
      mode.plots,
      isAdmin,
      reason,
      Math.round(player.position.x / 100),
    ])
    if (key === previewKey) return
    previewKey = key
    const samples = landscapingSamples(
      currentStroke,
      isAdmin ? null : mode.plots
    )
    const paintedSamples = samples.filter(
      (sample) => !sample.fringe && sample.weight > 0
    )
    canPaint = reason === null && paintedSamples.length > 0
    landscapingHint.set(
      reason ??
        (paintedSamples.length === 0
          ? 'Choose editable ground inside your estate'
          : null)
    )
    const positions: number[] = []
    const append = (x: number, z: number) =>
      positions.push(x, heightManager.getHeightAtWorldPosition(x, z) + 0.06, z)
    for (const sample of paintedSamples) {
      const x = unwrapWorldXNear(player.position.x, sample.x)
      const z = sample.z
      const half = sample.weight > 0.1 ? 0.46 : 0.25
      append(x - half, z - half)
      append(x + half, z - half)
      append(x + half, z + half)
      append(x - half, z - half)
      append(x + half, z + half)
      append(x - half, z + half)
    }
    preview.geometry.dispose()
    preview.geometry = new THREE.BufferGeometry()
    preview.geometry.setAttribute(
      'position',
      new THREE.Float32BufferAttribute(positions, 3)
    )
    preview.geometry.computeBoundingSphere()
    material.color.set(canPaint ? '#83dba1' : '#ed7265')
    preview.visible = paintedSamples.length > 0
  }

  function sendStroke() {
    if (get(landscapingPending)) return false
    const stroke = strokes.take()
    if (!stroke) return false
    landscapingPending.set(true)
    landscapingError.set(null)
    networkManager.sendEditLandscape(stroke)
    return true
  }

  onMount(() => {
    const canvas = renderer.domElement
    const unsubscribe = landscapingMode.subscribe(() => {
      painting = false
      strokes.clear()
      previewKey = ''
    })
    const unsubscribeHeight = heightManager.onHeightChanged(() => {
      previewKey = ''
    })
    const move = (event: PointerEvent) => {
      cursor = { x: event.clientX, y: event.clientY }
    }
    const leave = () => {
      strokes.finish()
      cursor = null
      painting = false
      preview.visible = false
    }
    const up = (event: PointerEvent) => {
      if (event.button !== 0 || !painting) return
      cursor = { x: event.clientX, y: event.clientY }
      updatePreview()
      if (currentStroke && canPaint) strokes.move(currentStroke)
      strokes.finish()
      painting = false
      sendStroke()
    }
    const down = (event: PointerEvent) => {
      const mode = get(landscapingMode)
      if (
        !mode ||
        mode.tool === 'Fence' ||
        event.button !== 0 ||
        get(cameraRotationEnabled)
      )
        return
      event.preventDefault()
      event.stopImmediatePropagation()
      cursor = { x: event.clientX, y: event.clientY }
      updatePreview()
      if (!currentStroke || !canPaint) return
      if (mode.tool === 'Road') {
        if (!get(landscapingRoadStart)) {
          landscapingRoadStart.set(currentStroke.start)
        } else {
          strokes.addRoad(currentStroke)
          sendStroke()
          landscapingRoadStart.set(null)
        }
      } else {
        painting = true
        strokes.begin(currentStroke)
        sendStroke()
      }
      elapsed = 0
    }
    canvas.addEventListener('pointermove', move)
    canvas.addEventListener('pointerleave', leave)
    canvas.addEventListener('pointercancel', leave)
    // Handle painting before OrbitControls captures the pointer on the wrapper.
    canvas.addEventListener('pointerdown', down, true)
    window.addEventListener('pointerup', up)
    window.addEventListener('blur', leave)
    return () => {
      unsubscribe()
      unsubscribeHeight()
      canvas.removeEventListener('pointermove', move)
      canvas.removeEventListener('pointerleave', leave)
      canvas.removeEventListener('pointercancel', leave)
      canvas.removeEventListener('pointerdown', down, true)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('blur', leave)
      preview.geometry.dispose()
      material.dispose()
    }
  })

  useTask((delta) => {
    updatePreview()
    if (painting) {
      if (currentStroke && canPaint) {
        strokes.move(currentStroke)
      } else {
        strokes.finish()
        painting = false
      }
    }
    elapsed += delta
    if (elapsed >= 0.1 && sendStroke()) {
      elapsed = 0
    }
  })
</script>

<T is={preview} />

<script lang="ts">
  import { T, useTask, useThrelte } from '@threlte/core'
  import { onMount } from 'svelte'
  import { get } from 'svelte/store'
  import * as THREE from 'three'
  import { loadGLB } from '../../utils/gltfCache'
  import { networkManager } from '../../network/socket'
  import {
    fences,
    fenceMode,
    fencePending,
    fenceError,
    fenceTarget,
    fenceCount,
    stopFenceMode,
    refreshFenceHeights,
  } from '../../stores/fenceStore'
  import {
    mapEditorMode,
    housingEditorMode,
    cameraRotationEnabled,
  } from '../../stores/debugStore'
  import { currentDungeonDepth } from '../../stores/dungeonStore'
  import { landscapingMode } from '../../stores/landscapingStore'
  import { playerTrade } from '../../stores/playerTradeStore'
  import { playerVisualFloorLevel } from '../../stores/housingStore'
  import {
    fenceCenter,
    fenceKey,
    fenceInReach,
    fenceOnOwnedPlot,
    nearestFenceEdge,
    type Fence,
    type FencePlot,
  } from '../../terrain/fenceEdges'
  import { unwrapWorldXNear, wrapWorldX } from '../../terrain/world-wrap'
  import { LAND_PLOT_SIZE } from '../../terrain/terrain-constants'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'
  import type { LocalPlayer } from '../../stores/gameStore'

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
  const group = new THREE.Group()
  const raycaster = new THREE.Raycaster()
  const pointer = new THREE.Vector2()
  const matrix = new THREE.Matrix4()
  const rotation = new THREE.Quaternion()
  const scale = new THREE.Vector3(1, 1, 1)
  const position = new THREE.Vector3()
  const up = new THREE.Vector3(0, 1, 0)
  let model: THREE.Mesh | null = null
  let instances: THREE.InstancedMesh | null = null
  let ghost: THREE.Mesh | null = null
  let entries: Fence[] = []
  let lastFences: Map<string, Fence> | null = null
  let lastWrapX = Infinity
  let cursor: { x: number; y: number } | null = null
  let disposed = false
  let lastGridPlots: FencePlot[] | null = null
  let lastGridWrapX = Infinity
  let gridDirty = true
  let fenceHeightsDirty = false
  const gridMaterial = new THREE.LineDashedMaterial({
    color: '#b7ae93',
    transparent: true,
    opacity: 0.2,
    dashSize: 0.15,
    gapSize: 0.1,
    depthTest: false,
  })
  const grid = new THREE.LineSegments(new THREE.BufferGeometry(), gridMaterial)
  grid.visible = false
  grid.renderOrder = 10
  group.add(grid)

  function updateGrid() {
    const mode = get(landscapingMode)
    grid.visible = !!mode && !!player
    if (!mode || !player) return
    if (
      mode.plots === lastGridPlots &&
      Math.abs(player.position.x - lastGridWrapX) < 100 &&
      !gridDirty
    )
      return
    lastGridPlots = mode.plots
    lastGridWrapX = player.position.x
    gridDirty = false
    const positions: number[] = []
    const plots = new Set(mode.plots.map((plot) => `${plot.x},${plot.z}`))
    const segment = (x1: number, z1: number, x2: number, z2: number) => {
      const y1 = heightManager.groundYOrNull(wrapWorldX(x1), z1)
      const y2 = heightManager.groundYOrNull(wrapWorldX(x2), z2)
      if (y1 === null || y2 === null) return
      positions.push(x1, y1 + 0.04, z1, x2, y2 + 0.04, z2)
    }
    for (const plot of mode.plots) {
      const x = unwrapWorldXNear(player.position.x, plot.x)
      const west = plots.has(`${wrapWorldX(plot.x - LAND_PLOT_SIZE)},${plot.z}`)
      const south = plots.has(`${plot.x},${plot.z - LAND_PLOT_SIZE}`)
      for (let offset = 0; offset <= LAND_PLOT_SIZE; offset++) {
        for (let step = 0; step < LAND_PLOT_SIZE; step++) {
          if (offset > 0 || !west)
            segment(x + offset, plot.z + step, x + offset, plot.z + step + 1)
          if (offset > 0 || !south)
            segment(x + step, plot.z + offset, x + step + 1, plot.z + offset)
        }
      }
    }
    grid.geometry.dispose()
    grid.geometry = new THREE.BufferGeometry()
    grid.geometry.setAttribute(
      'position',
      new THREE.Float32BufferAttribute(positions, 3)
    )
    grid.computeLineDistances()
    grid.geometry.computeBoundingSphere()
  }

  function rebuild() {
    const current = get(fences)
    if (
      !model ||
      !player ||
      (current === lastFences && Math.abs(player.position.x - lastWrapX) < 100)
    )
      return
    lastFences = current
    lastWrapX = player.position.x
    entries = [...current.values()]
    if (!instances || instances.instanceMatrix.count < entries.length) {
      const capacity = Math.max(
        1,
        entries.length,
        (instances?.instanceMatrix.count ?? 0) * 2
      )
      if (instances) {
        group.remove(instances)
        instances.dispose()
      }
      instances = new THREE.InstancedMesh(
        model.geometry,
        model.material,
        capacity
      )
      instances.castShadow = true
      instances.receiveShadow = true
      group.add(instances)
    }
    instances.count = entries.length
    for (const [index, fence] of entries.entries()) {
      const center = fenceCenter(fence.edge)
      position.set(
        unwrapWorldXNear(player.position.x, center.x),
        fence.y,
        center.z
      )
      rotation.setFromAxisAngle(up, fence.edge.axis === 'Z' ? Math.PI / 2 : 0)
      matrix.compose(position, rotation, scale)
      instances.setMatrixAt(index, matrix)
    }
    instances.instanceMatrix.needsUpdate = true
    instances.computeBoundingSphere()
  }

  function hidePreview() {
    if (ghost) ghost.visible = false
    fenceTarget.set(null)
  }

  function updatePreview() {
    const mode = get(fenceMode)
    if (!mode || !player || !cursor || !ghost || get(cameraRotationEnabled)) {
      hidePreview()
      return
    }
    const canvas = renderer.domElement
    const rect = canvas.getBoundingClientRect()
    pointer.set(
      ((cursor.x - rect.left) / rect.width) * 2 - 1,
      -((cursor.y - rect.top) / rect.height) * 2 + 1
    )
    raycaster.setFromCamera(pointer, get(camera))
    const grounds = terrainMeshes.filter((m): m is THREE.Mesh => !!m)
    const groundHit = raycaster.intersectObjects(grounds, false)[0]
    const fenceHit = instances
      ? raycaster.intersectObject(instances, false)[0]
      : undefined
    const hitFence =
      fenceHit?.instanceId !== undefined &&
      (!groundHit || fenceHit.distance <= groundHit.distance)
        ? entries[fenceHit.instanceId]
        : undefined
    if (!hitFence && !groundHit) {
      hidePreview()
      return
    }
    const edge =
      hitFence?.edge ?? nearestFenceEdge(groundHit.point.x, groundHit.point.z)
    const existing = get(fences).get(fenceKey(edge))
    const center = fenceCenter(edge)
    const x = unwrapWorldXNear(player.position.x, center.x)
    const heightAt = (t: number) =>
      heightManager.getHeightAtWorldPosition(
        x + (edge.axis === 'X' ? t : 0),
        center.z + (edge.axis === 'Z' ? t : 0)
      )
    const heights = [-0.5, 0, 0.5].map(heightAt)
    const y = existing?.y ?? Math.min(...heights)
    const reason =
      existing && existing.owner_id !== mode.owner_id
        ? 'This fence belongs to another player'
        : !existing && !fenceOnOwnedPlot(edge, mode.plots)
          ? 'Choose an edge on your estate'
          : !existing && !get(fenceCount)
            ? 'No fences left · Click a placed fence to recover it'
            : !existing && Math.max(...heights) - Math.min(...heights) > 0.5
              ? 'This edge is too steep'
              : null
    ghost.visible = true
    ghost.position.set(x, y, center.z)
    ghost.rotation.y = edge.axis === 'Z' ? Math.PI / 2 : 0
    const material = ghost.material as THREE.MeshStandardMaterial
    material.color.set(reason ? '#ff4444' : existing ? '#ffbb55' : '#60ff90')
    fenceTarget.set({
      edge,
      removing: !!existing,
      valid: reason === null,
      reason,
    })
  }

  onMount(() => {
    const unsubscribeHeight = heightManager.onHeightChanged(() => {
      gridDirty = true
      fenceHeightsDirty = true
    })
    loadGLB('/models/objects/wooden_fence.glb')
      .then((gltf) => {
        if (disposed) return
        gltf.scene.updateMatrixWorld(true)
        gltf.scene.traverse((object) => {
          if (model || !(object instanceof THREE.Mesh)) return
          model = object
        })
        if (!model) return
        const source = model as THREE.Mesh
        const material = (
          Array.isArray(source.material) ? source.material[0] : source.material
        ).clone() as THREE.MeshStandardMaterial
        material.transparent = true
        material.opacity = 0.65
        material.depthWrite = false
        ghost = new THREE.Mesh(source.geometry, material)
        ghost.visible = false
        ghost.renderOrder = 5
        group.add(ghost)
        rebuild()
      })
      .catch((error) => {
        console.error('Failed to load wooden fence:', error)
        fenceError.set('Could not load the fence preview. Reload to try again.')
      })
    const canvas = renderer.domElement
    const move = (event: PointerEvent) => {
      cursor = { x: event.clientX, y: event.clientY }
    }
    const leave = (event: PointerEvent) => {
      if (
        event.relatedTarget instanceof Node &&
        event.relatedTarget.contains(canvas)
      )
        return
      cursor = null
      hidePreview()
    }
    const click = (event: MouseEvent) => {
      if (get(cameraRotationEnabled)) return
      if (get(landscapingMode) && !get(fenceMode)) return
      if (!get(fenceMode)) {
        if (
          event.button !== 2 ||
          !instances ||
          !player ||
          get(playerVisualFloorLevel) !== 0 ||
          get(mapEditorMode) ||
          get(housingEditorMode)
        )
          return
        const rect = canvas.getBoundingClientRect()
        pointer.set(
          ((event.clientX - rect.left) / rect.width) * 2 - 1,
          -((event.clientY - rect.top) / rect.height) * 2 + 1
        )
        raycaster.setFromCamera(pointer, get(camera))
        const hit = raycaster.intersectObject(instances, false)[0]
        if (
          hit?.instanceId === undefined ||
          !fenceInReach(entries[hit.instanceId].edge, player.position)
        )
          return
        event.preventDefault()
        event.stopImmediatePropagation()
        networkManager.sendStartFenceMode()
        return
      }
      if (event.button !== 0) return
      event.preventDefault()
      event.stopImmediatePropagation()
      cursor = { x: event.clientX, y: event.clientY }
      updatePreview()
      const target = get(fenceTarget)
      if (!target?.valid || get(fencePending)) return
      fencePending.set(true)
      fenceError.set(null)
      networkManager.sendEditFence(target.edge, !target.removing)
    }
    const escape = (event: KeyboardEvent) => {
      if (event.code !== 'Escape' || !get(landscapingMode)) return
      event.preventDefault()
      event.stopImmediatePropagation()
      stopFenceMode()
      hidePreview()
    }
    canvas.addEventListener('pointermove', move)
    canvas.addEventListener('pointerleave', leave)
    canvas.addEventListener('mousedown', click, true)
    window.addEventListener('keydown', escape, true)
    return () => {
      disposed = true
      unsubscribeHeight()
      canvas.removeEventListener('pointermove', move)
      canvas.removeEventListener('pointerleave', leave)
      canvas.removeEventListener('mousedown', click, true)
      window.removeEventListener('keydown', escape, true)
      instances?.dispose()
      if (ghost) (ghost.material as THREE.Material).dispose()
      grid.geometry.dispose()
      gridMaterial.dispose()
      stopFenceMode()
    }
  })

  let elapsed = 0
  useTask((delta) => {
    group.visible = get(currentDungeonDepth) < 1
    if (
      get(landscapingMode) &&
      (!player ||
        player.health <= 0 ||
        get(playerVisualFloorLevel) !== 0 ||
        get(currentDungeonDepth) > 0 ||
        get(mapEditorMode) ||
        get(housingEditorMode) ||
        get(playerTrade))
    )
      stopFenceMode()
    if (fenceHeightsDirty) {
      fenceHeightsDirty = false
      refreshFenceHeights((x, z) => heightManager.groundYOrNull(x, z))
    }
    rebuild()
    updateGrid()
    elapsed += delta
    if (elapsed >= 0.05) {
      elapsed = 0
      updatePreview()
    }
  })
</script>

<T is={group} />

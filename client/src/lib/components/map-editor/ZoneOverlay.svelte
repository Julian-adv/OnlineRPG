<script lang="ts">
  import * as THREE from 'three'
  import { T } from '@threlte/core'
  import { onDestroy } from 'svelte'
  import {
    editorTool,
    zoneDrawStart,
    editorHeightManager,
    hoveredCell,
    hoveredZoneIndex,
    currentZoneData,
  } from '../../stores/editorStore'
  import type { ZoneData } from '../../managers/zoneManager'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'

  let {
    data,
    readOnly = false,
  }: {
    data?: ZoneData
    readOnly?: boolean
  } = $props()

  const NO_SPAWN_COLOR = new THREE.Color(0xff4444)
  const SPAWN_COLOR = new THREE.Color(0x4488ff)
  const PREVIEW_COLOR = new THREE.Color(0xffcc00)
  const HIGHLIGHT_COLOR = new THREE.Color(0xffffff)
  const Y_OFFSET = 0.15

  interface ZoneMesh {
    key: string
    color: THREE.Color
    highlighted: boolean
    fill: THREE.BufferGeometry | null
    border: THREE.BufferGeometry | null
  }

  const zoneData = $derived(data ?? $currentZoneData)
  const drawStart = $derived($zoneDrawStart)
  const cursorPos = $derived($hoveredCell)
  const tool = $derived($editorTool)
  const heightMgr = $derived($editorHeightManager)
  const hovered = $derived(readOnly ? null : $hoveredZoneIndex)
  let heightRevision = $state(0)

  $effect(() =>
    heightMgr?.onHeightChanged(() => {
      heightRevision++
    })
  )

  function buildFillGeometry(
    mgr: TerrainHeightManager,
    minX: number,
    minZ: number,
    maxX: number,
    maxZ: number
  ): THREE.BufferGeometry | null {
    const cellMinX = Math.floor(minX)
    const cellMinZ = Math.floor(minZ)
    const cellMaxX = Math.ceil(maxX)
    const cellMaxZ = Math.ceil(maxZ)
    const cols = cellMaxX - cellMinX
    const rows = cellMaxZ - cellMinZ
    if (cols <= 0 || rows <= 0) return null

    const vertCols = cols + 1
    const positions = new Float32Array((cols + 1) * (rows + 1) * 3)

    for (let r = 0; r <= rows; r++) {
      for (let c = 0; c <= cols; c++) {
        const wx = cellMinX + c
        const wz = cellMinZ + r
        const idx = (r * vertCols + c) * 3
        positions[idx] = wx
        positions[idx + 1] = mgr.getHeightAtWorldPosition(wx, wz) + Y_OFFSET
        positions[idx + 2] = wz
      }
    }

    const indices = new Uint32Array(cols * rows * 6)
    let ii = 0
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const tl = r * vertCols + c
        const tr = tl + 1
        const bl = (r + 1) * vertCols + c
        const br = bl + 1
        indices[ii++] = tl
        indices[ii++] = bl
        indices[ii++] = tr
        indices[ii++] = tr
        indices[ii++] = bl
        indices[ii++] = br
      }
    }

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setIndex(new THREE.BufferAttribute(indices, 1))
    return geo
  }

  function buildBorderGeometry(
    mgr: TerrainHeightManager,
    minX: number,
    minZ: number,
    maxX: number,
    maxZ: number
  ): THREE.BufferGeometry | null {
    const points: number[] = []
    const yOff = Y_OFFSET + 0.05

    function addEdge(x0: number, z0: number, x1: number, z1: number) {
      const dx = x1 - x0
      const dz = z1 - z0
      const len = Math.sqrt(dx * dx + dz * dz)
      const count = Math.max(1, Math.ceil(len))
      for (let i = 0; i < count; i++) {
        const t = i / count
        const x = x0 + dx * t
        const z = z0 + dz * t
        points.push(x, mgr.getHeightAtWorldPosition(x, z) + yOff, z)
      }
    }

    addEdge(minX, minZ, maxX, minZ)
    addEdge(maxX, minZ, maxX, maxZ)
    addEdge(maxX, maxZ, minX, maxZ)
    addEdge(minX, maxZ, minX, minZ)

    if (points.length === 0) return null
    const geo = new THREE.BufferGeometry()
    geo.setAttribute(
      'position',
      new THREE.Float32BufferAttribute(new Float32Array(points), 3)
    )
    return geo
  }

  function disposeGeos(list: ZoneMesh[]) {
    for (const m of list) {
      m.fill?.dispose()
      m.border?.dispose()
    }
  }

  let prevStaticGeos: ZoneMesh[] = []

  let staticMeshes = $derived.by(() => {
    if ((!readOnly && tool !== 'zone') || !heightMgr) {
      disposeGeos(prevStaticGeos)
      prevStaticGeos = []
      return []
    }

    const mgr = heightMgr
    void heightRevision
    const result: ZoneMesh[] = []

    const noSpawnZones = zoneData.noSpawnZones ?? []
    for (let i = 0; i < noSpawnZones.length; i++) {
      const z = noSpawnZones[i]
      const hi = hovered?.type === 'noSpawn' && hovered.index === i
      result.push({
        key: `ns-${i}`,
        color: hi ? HIGHLIGHT_COLOR : NO_SPAWN_COLOR,
        highlighted: hi,
        fill: buildFillGeometry(mgr, z.minX, z.minZ, z.maxX, z.maxZ),
        border: buildBorderGeometry(mgr, z.minX, z.minZ, z.maxX, z.maxZ),
      })
    }

    const spawnZones = readOnly ? [] : (zoneData.monsterSpawns ?? [])
    for (let i = 0; i < spawnZones.length; i++) {
      const z = spawnZones[i]
      const hi = hovered?.type === 'spawn' && hovered.index === i
      result.push({
        key: `sp-${i}`,
        color: hi ? HIGHLIGHT_COLOR : SPAWN_COLOR,
        highlighted: hi,
        fill: buildFillGeometry(mgr, z.minX, z.minZ, z.maxX, z.maxZ),
        border: buildBorderGeometry(mgr, z.minX, z.minZ, z.maxX, z.maxZ),
      })
    }

    disposeGeos(prevStaticGeos)
    prevStaticGeos = result
    return result
  })

  let prevPreviewGeos: ZoneMesh | null = null

  let previewMesh = $derived.by((): ZoneMesh | null => {
    if (readOnly || tool !== 'zone' || !heightMgr || !drawStart || !cursorPos) {
      if (prevPreviewGeos) {
        prevPreviewGeos.fill?.dispose()
        prevPreviewGeos.border?.dispose()
        prevPreviewGeos = null
      }
      return null
    }

    const minX = Math.min(drawStart.x, cursorPos.worldX)
    const minZ = Math.min(drawStart.z, cursorPos.worldZ)
    const maxX = Math.max(drawStart.x, cursorPos.worldX)
    const maxZ = Math.max(drawStart.z, cursorPos.worldZ)
    if (maxX - minX < 0.5 || maxZ - minZ < 0.5) return null

    if (prevPreviewGeos) {
      prevPreviewGeos.fill?.dispose()
      prevPreviewGeos.border?.dispose()
    }

    const mgr = heightMgr
    const m: ZoneMesh = {
      key: 'preview',
      color: PREVIEW_COLOR,
      highlighted: false,
      fill: buildFillGeometry(mgr, minX, minZ, maxX, maxZ),
      border: buildBorderGeometry(mgr, minX, minZ, maxX, maxZ),
    }
    prevPreviewGeos = m
    return m
  })

  onDestroy(() => {
    disposeGeos(prevStaticGeos)
    if (prevPreviewGeos) {
      prevPreviewGeos.fill?.dispose()
      prevPreviewGeos.border?.dispose()
    }
  })
</script>

{#each staticMeshes as m (m.key)}
  {#if m.fill}
    <T.Mesh renderOrder={999} frustumCulled={false}>
      <T is={m.fill} />
      <T.MeshBasicMaterial
        color={m.color}
        transparent={true}
        opacity={m.highlighted ? 0.45 : 0.2}
        side={THREE.DoubleSide}
        depthWrite={false}
      />
    </T.Mesh>
  {/if}
  {#if m.border}
    <T.LineLoop renderOrder={999} frustumCulled={false}>
      <T is={m.border} />
      <T.LineBasicMaterial color={m.color} linewidth={2} depthWrite={false} />
    </T.LineLoop>
  {/if}
{/each}

{#if previewMesh}
  {#if previewMesh.fill}
    <T.Mesh renderOrder={999} frustumCulled={false}>
      <T is={previewMesh.fill} />
      <T.MeshBasicMaterial
        color={PREVIEW_COLOR}
        transparent={true}
        opacity={0.25}
        side={THREE.DoubleSide}
        depthWrite={false}
      />
    </T.Mesh>
  {/if}
  {#if previewMesh.border}
    <T.LineLoop renderOrder={999} frustumCulled={false}>
      <T is={previewMesh.border} />
      <T.LineBasicMaterial
        color={PREVIEW_COLOR}
        linewidth={2}
        depthWrite={false}
      />
    </T.LineLoop>
  {/if}
{/if}

<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { SvelteMap } from 'svelte/reactivity'
  import { onMount } from 'svelte'
  import { WebGPURenderer } from 'three/webgpu'
  import type { TerrainTile } from './terrain-utils'
  import { TERRAIN_TILE_SIZE, parseTileId } from './terrain-utils'
  import type {
    TerrainHeightManager,
    AffectedTile,
  } from '../../managers/terrainHeightManager'
  import type { WaterFieldManager } from '../../managers/waterFieldManager'
  import type { WaterFieldTileData } from '../../utils/water-field-data'
  import {
    createWaterQuadGeometry,
    applyWaterFieldToGeometry,
    buildWaterFieldTexture,
    waterFieldFallbackTex,
    getSharedSeaGeometry,
  } from '../../utils/water-quad-geometry'
  import {
    createWaterFieldMaterial,
    getWaterCaptureMaterial,
    type WaterFieldMaterialResult,
  } from '../../shaders/water-field-material'
  import {
    waterHeightFallbackTex,
    waterWetnessFallbackTex,
  } from '../../shaders/water-types'
  import {
    createWetnessSystem,
    type WetnessResult,
  } from '../../shaders/wetness-compute'
  import {
    getAppliedAntialias,
    getCurrentPreset,
  } from '../../stores/graphicsSettings'
  import { riverWireframeVisible } from '../../stores/debugStore'
  import { enqueueTileWork } from '../../utils/tileWorkQueue'

  interface Props {
    terrainTiles: TerrainTile[]
    heightManager?: TerrainHeightManager | null
    waterFieldManager?: WaterFieldManager | null
    normalMap?: THREE.Texture | null
    foamMap?: THREE.Texture | null
    causticsMap?: THREE.Texture | null
    time?: number
    sunDirection?: THREE.Vector3 | null
    sunColor?: THREE.Color | null
    cameraDirection?: THREE.Vector3 | null
    moonBrightness?: number
    refractionMap?: THREE.Texture | null
    reflectionMap?: THREE.Texture | null
    torchLight?: THREE.PointLight | null
    waterGroup?: THREE.Group | undefined
  }

  let {
    terrainTiles,
    heightManager = null,
    waterFieldManager = null,
    normalMap = null,
    foamMap = null,
    causticsMap = null,
    time = 0,
    sunDirection = null,
    sunColor = null,
    cameraDirection = null,
    moonBrightness = 0,
    refractionMap = null,
    reflectionMap = null,
    torchLight = null,
    waterGroup = $bindable(undefined),
  }: Props = $props()

  // Per-pixel depth shoreline is off when the canvas is multisampled —
  // the viewportDepthTexture copy can't source an MSAA depth buffer — and
  // on presets that can't afford that copy's per-frame cost.
  const pixelDepth =
    getCurrentPreset().waterPixelDepth && !getAppliedAntialias()

  const group = new THREE.Group()
  group.name = 'water'
  waterGroup = group

  export function getGroup(): THREE.Group {
    return group
  }

  /** True when at least one streamed tile actually has water. Gates the
   *  refraction/reflection passes — inland there is nothing to sample them. */
  export function hasActiveWater(): boolean {
    return matMap.size > 0
  }

  // ── Per-tile state ──
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const meshes = new Map<string, THREE.Mesh>()
  // Heightmap textures are intentionally never disposed — see the
  // Sampler-stale-binding note in releaseTile.
  const heightTexMap = new SvelteMap<string, THREE.DataTexture>()
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const fieldTexMap = new Map<string, THREE.DataTexture | null>()
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const geometryMap = new Map<string, THREE.BufferGeometry | null>()
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const matMap = new Map<string, WaterFieldMaterialResult>()
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const wetnessMap = new Map<string, WetnessResult>()
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const noWaterSet = new Set<string>()
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const inflightTiles = new Set<string>()

  // ── Pools (reused across tile lifecycles — one pipeline compile each) ──
  const matPool: WaterFieldMaterialResult[] = []
  const wetnessPool: WetnessResult[] = []

  const wireframeMaterial = new THREE.LineBasicMaterial({
    color: 0xff0000,
    transparent: true,
    opacity: 0.9,
    depthTest: true,
    depthWrite: false,
  })
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const wireframeMeshes = new Map<string, THREE.LineSegments>()

  function acquireMaterial(): WaterFieldMaterialResult | null {
    const pooled = matPool.pop()
    if (pooled) return pooled
    if (!normalMap || !foamMap || !causticsMap) return null
    return createWaterFieldMaterial({
      heightmapTexture: waterHeightFallbackTex,
      waterField: waterFieldFallbackTex,
      normalMap,
      foamMap,
      causticsMap,
      refractionMap,
      reflectionMap,
      pixelDepth,
    })
  }

  function addWireframeForTile(id: string, mesh: THREE.Mesh) {
    if (wireframeMeshes.has(id)) return
    const wf = new THREE.LineSegments(
      new THREE.WireframeGeometry(mesh.geometry),
      wireframeMaterial
    )
    wf.renderOrder = 10
    wf.castShadow = false
    wf.receiveShadow = false
    mesh.add(wf)
    wireframeMeshes.set(id, wf)
  }

  function removeWireframeForTile(id: string) {
    const wf = wireframeMeshes.get(id)
    if (!wf) return
    wf.removeFromParent()
    wf.geometry.dispose()
    wireframeMeshes.delete(id)
  }

  $effect(() => {
    if ($riverWireframeVisible) {
      for (const [id, mesh] of meshes) addWireframeForTile(id, mesh)
    } else {
      for (const id of [...wireframeMeshes.keys()]) removeWireframeForTile(id)
    }
  })

  function releaseTile(id: string) {
    const mesh = meshes.get(id)
    if (mesh) {
      removeWireframeForTile(id)
      group.remove(mesh)
      meshes.delete(id)
    }
    const mat = matMap.get(id)
    if (mat) {
      // Reset pooled-material textures to shared fallbacks BEFORE the
      // per-tile textures go out of scope. Never dispose textures that
      // were bound to a pooled material: three's Sampler binding listens
      // for 'dispose' and nullifies .texture, and _init doesn't re-sync
      // bindings — a re-pooled material could hit createBindGroup with a
      // null texture and crash. GC reclaims them (a handful at a time).
      mat.uniforms.uHeightmapTexture.value = waterHeightFallbackTex
      mat.uniforms.uWaterField.value = waterFieldFallbackTex
      matMap.delete(id)
      matPool.push(mat)
    }
    const wetness = wetnessMap.get(id)
    if (wetness) {
      wetnessMap.delete(id)
      wetnessPool.push(wetness)
    }
    // Per-tile geometry is only referenced by this tile's mesh + capture
    // mesh (which reposition() re-points on pool reuse) — safe to dispose.
    // The shared sea geometry (stored as null) must never be disposed.
    const geometry = geometryMap.get(id)
    if (geometry) geometry.dispose()
    geometryMap.delete(id)
    heightTexMap.delete(id)
    fieldTexMap.delete(id)
  }

  function acquireWetness(
    id: string,
    tileX: number,
    tileZ: number,
    geometry: THREE.BufferGeometry
  ) {
    const pooled = wetnessPool.pop()
    if (pooled) {
      pooled.reposition(tileX, tileZ, geometry)
      wetnessMap.set(id, pooled)
    } else {
      wetnessMap.set(
        id,
        createWetnessSystem(geometry, tileX, tileZ, TERRAIN_TILE_SIZE)
      )
    }
  }

  function activateTile(
    id: string,
    tileX: number,
    tileZ: number,
    field: WaterFieldTileData | null
  ) {
    if (!heightManager || matMap.has(id)) return
    const hasSea = heightManager.hasWater(tileX, tileZ)
    const hasWater = hasSea || field !== null
    if (!hasWater) {
      noWaterSet.add(id)
      return
    }

    const heightTex = heightManager.getHeightmapTexture(tileX, tileZ)
    if (!heightTex) return
    const mat = acquireMaterial()
    if (!mat) return // shared textures not ready yet

    const geometry = field ? createWaterQuadGeometry() : getSharedSeaGeometry()
    if (field) applyWaterFieldToGeometry(geometry, field)
    const fieldTex = field ? buildWaterFieldTexture(field) : null

    const u = mat.uniforms
    u.uHeightmapTexture.value = heightTex
    u.uWaterField.value = fieldTex ?? waterFieldFallbackTex
    if (refractionMap) u.uRefractionMap.value = refractionMap
    if (reflectionMap) u.uReflectionMap.value = reflectionMap

    // Wetness only darkens wet sand along a shoreline. Rivers don't wet their
    // banks — the capture material zeroes river influence outright — so a tile
    // with no sea in it would spend two render passes every other frame
    // producing an empty texture.
    if (hasSea) acquireWetness(id, tileX, tileZ, geometry)

    const mesh = new THREE.Mesh(geometry, mat.material)
    mesh.position.set(tileX * TERRAIN_TILE_SIZE, 0, tileZ * TERRAIN_TILE_SIZE)
    mesh.receiveShadow = false
    mesh.castShadow = false
    mesh.renderOrder = 0
    group.add(mesh)

    meshes.set(id, mesh)
    matMap.set(id, mat)
    heightTexMap.set(id, heightTex)
    fieldTexMap.set(id, fieldTex)
    geometryMap.set(id, field ? geometry : null)
    noWaterSet.delete(id)

    if ($riverWireframeVisible) addWireframeForTile(id, mesh)
  }

  async function loadTile(id: string, tileX: number, tileZ: number) {
    if (inflightTiles.has(id) || matMap.has(id) || noWaterSet.has(id)) return
    if (!heightManager || !waterFieldManager) return
    inflightTiles.add(id)
    try {
      const [, field] = await Promise.all([
        heightManager.loadHeightmap(tileX, tileZ).catch(() => null),
        waterFieldManager.loadWaterField(tileX, tileZ),
      ])
      // Route through the work queue to prevent clustering when data is cached
      enqueueTileWork(() => activateTile(id, tileX, tileZ, field))
    } finally {
      inflightTiles.delete(id)
    }
  }

  /** Height data changed or first loaded: refresh the heightmap texture in
   *  place (no material recompile), and re-evaluate water presence for
   *  inactive tiles (loadTile's inflight/active guards absorb load events). */
  function refreshTile(id: string, tileX: number, tileZ: number) {
    if (!heightManager) return
    const heightTex = heightTexMap.get(id)
    if (heightTex) {
      heightManager.updateHeightmapTexture(tileX, tileZ, heightTex)
      // A sea-only tile can lose its last sub-sea vertex to an edit.
      if (
        fieldTexMap.get(id) === null &&
        !heightManager.hasWater(tileX, tileZ)
      ) {
        releaseTile(id)
        noWaterSet.add(id)
        return
      }
      // An edit can also carry a river tile across sea level in either
      // direction, and only sea-bearing tiles carry a wetness system.
      const hasSea = heightManager.hasWater(tileX, tileZ)
      const wetness = wetnessMap.get(id)
      if (hasSea && !wetness) {
        const geometry = geometryMap.get(id) ?? meshes.get(id)?.geometry
        if (geometry) acquireWetness(id, tileX, tileZ, geometry)
      } else if (!hasSea && wetness) {
        wetnessMap.delete(id)
        wetnessPool.push(wetness)
      }
    } else {
      // Not active — an edit may have dug below sea level.
      noWaterSet.delete(id)
      void loadTile(id, tileX, tileZ)
    }
  }

  function refreshAdjacentTiles(tileX: number, tileZ: number) {
    // Adjacent tiles whose 65th edge row/column references this tile's
    // height data (mirrors refreshAdjacentTileEdges on the terrain side).
    const neighbors = [
      { dx: -1, dz: 0 },
      { dx: 0, dz: -1 },
      { dx: -1, dz: -1 },
    ]
    for (const { dx, dz } of neighbors) {
      const nx = tileX + dx
      const nz = tileZ + dz
      const nid = `${nx}_${nz}`
      if (heightTexMap.has(nid)) refreshTile(nid, nx, nz)
    }
  }

  onMount(() => {
    if (!heightManager) return
    const unsub = heightManager.onHeightChanged((tiles: AffectedTile[]) => {
      for (const { tileX, tileZ } of tiles) {
        refreshTile(`${tileX}_${tileZ}`, tileX, tileZ)
        refreshAdjacentTiles(tileX, tileZ)
      }
    })
    return unsub
  })

  // ── Tile list changes ──
  $effect(() => {
    if (
      !heightManager ||
      !waterFieldManager ||
      !normalMap ||
      !foamMap ||
      !causticsMap
    )
      return

    const currentIds = new Set(terrainTiles.map((t) => t.id))
    for (const id of [...matMap.keys()]) {
      if (!currentIds.has(id)) releaseTile(id)
    }
    for (const id of [...noWaterSet]) {
      if (!currentIds.has(id)) noWaterSet.delete(id)
    }
    for (const tile of terrainTiles) {
      const coords = parseTileId(tile.id)
      if (!coords) continue
      void loadTile(tile.id, coords.tileX, coords.tileZ)
    }
  })

  // ── Per-frame: uniforms + wetness pre-pass ──
  let wetnessFrameCount = 0
  export function renderWetness(renderer: WebGPURenderer) {
    wetnessFrameCount++
    const doCapture = wetnessFrameCount % 2 === 0
    const capture = getWaterCaptureMaterial()
    capture.uniforms.uTime.value = time

    for (const [id, mat] of matMap) {
      const u = mat.uniforms
      u.uTime.value = time
      if (sunDirection) u.uSunDirection.value.copy(sunDirection)
      if (sunColor) u.uSunColor.value.copy(sunColor)
      if (cameraDirection) u.uCameraDirection.value.copy(cameraDirection)
      u.uMoonBrightness.value = moonBrightness
      if (refractionMap) u.uRefractionMap.value = refractionMap
      if (reflectionMap) u.uReflectionMap.value = reflectionMap
      if (torchLight) {
        u.uTorchPos.value.copy(torchLight.position)
        u.uTorchColor.value.copy(torchLight.color)
        u.uTorchIntensity.value = torchLight.intensity
        u.uTorchDistance.value = torchLight.distance
      } else {
        u.uTorchIntensity.value = 0
      }

      const wetness = wetnessMap.get(id)
      if (!wetness) {
        // River-only tile. Materials are pooled, so without this it would keep
        // sampling whichever tile's wetness target the material last held.
        u.uWetnessMap.value = waterWetnessFallbackTex
        continue
      }
      // Capture + decay only every other frame (wetness changes slowly;
      // the decay formula uses actual dt so skipping frames is safe).
      if (doCapture) {
        const heightTex = heightTexMap.get(id)
        if (heightTex) capture.uniforms.uHeightmapTexture.value = heightTex
        capture.uniforms.uWaterField.value =
          fieldTexMap.get(id) ?? waterFieldFallbackTex
        wetness.update(renderer, capture.material, time)
      }
      u.uWetnessMap.value = wetness.readTexture
    }
  }
</script>

<T is={group} />

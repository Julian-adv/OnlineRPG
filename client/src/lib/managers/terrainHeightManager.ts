import * as THREE from 'three'
import { getTerrainApiUrl } from '../utils/networkUtils'
import {
  TERRAIN_TILE_SIZE,
  SEA_LEVEL_ENCODED,
} from '../components/game-scene/terrain-utils'
import {
  VERTS_PER_SIDE,
  tileKey,
  decodeHeight,
  worldToTileCoord,
  type TerrainHeightState,
  type AffectedTile,
  type HeightChangedCallback,
} from './terrain-height-types'
import {
  getHeightAtCell,
  applyHeightToGeometry as applyHeightToGeo,
  refreshAdjacentTileEdges as doRefreshAdjacentEdges,
} from './terrain-height-geometry'
import {
  applyBrush as doBrush,
  applyFlatten as doFlatten,
  applyFlattenLine as doFlattenLine,
  flattenArea as doFlattenArea,
  flattenRotatedRect as doFlattenRotatedRect,
  restoreFromOriginal as doRestore,
} from './terrain-height-brushes'
import {
  loadHeightmap as doLoad,
  loadOriginalHeightmap as doLoadOriginal,
  ensureOriginalHeightmap as doEnsureOriginal,
  saveDirtyTiles,
} from './terrain-height-persistence'

export type { AffectedTile, HeightChangedCallback }

export class TerrainHeightManager {
  private state: TerrainHeightState = {
    heightmaps: new Map(),
    originalHeightmaps: new Map(),
    missingOriginalTiles: new Set(),
    geometries: new Map(),
    dirtyTiles: new Set(),
    dirtyOriginalTiles: new Set(),
  }
  private inflightHeightmaps = new Map<string, Promise<Uint16Array>>()
  private saveTimer: ReturnType<typeof setTimeout> | null = null
  private terrainApiUrl: string
  private heightChangedListeners = new Set<HeightChangedCallback>()

  constructor() {
    this.terrainApiUrl = getTerrainApiUrl()
  }

  onHeightChanged(cb: HeightChangedCallback): () => void {
    this.heightChangedListeners.add(cb)
    return () => {
      this.heightChangedListeners.delete(cb)
    }
  }

  private notifyHeightChanged(tiles: AffectedTile[]) {
    for (const cb of this.heightChangedListeners) cb(tiles)
  }

  private scheduleSave() {
    if (this.saveTimer !== null) {
      clearTimeout(this.saveTimer)
    }
    this.saveTimer = setTimeout(() => {
      saveDirtyTiles(this.state, this.terrainApiUrl)
      this.saveTimer = null
    }, 1000)
  }

  private finalize(affected: AffectedTile[]) {
    if (affected.length > 0) {
      this.scheduleSave()
      this.notifyHeightChanged(affected)
    }
  }

  // --- Data loading ---

  async loadHeightmap(tileX: number, tileZ: number): Promise<Uint16Array> {
    return doLoad(
      this.state,
      this.inflightHeightmaps,
      this.terrainApiUrl,
      tileX,
      tileZ,
      (tx, tz) => this.loadOriginalHeightmap(tx, tz),
      () => this.notifyHeightChanged([{ tileX, tileZ }])
    )
  }

  async loadOriginalHeightmap(
    tileX: number,
    tileZ: number
  ): Promise<Uint16Array | null> {
    return doLoadOriginal(this.state, this.terrainApiUrl, tileX, tileZ)
  }

  ensureOriginalHeightmap(tileX: number, tileZ: number): void {
    doEnsureOriginal(this.state, this.terrainApiUrl, tileX, tileZ)
  }

  // --- Height queries ---

  getHeightmap(tileX: number, tileZ: number): Uint16Array | undefined {
    return this.state.heightmaps.get(tileKey(tileX, tileZ))
  }

  getHeightAtCell(
    tileX: number,
    tileZ: number,
    cellX: number,
    cellZ: number
  ): number {
    return getHeightAtCell(this.state, tileX, tileZ, cellX, cellZ)
  }

  hasHeightData(worldX: number, worldZ: number): boolean {
    return this.state.heightmaps.has(
      tileKey(worldToTileCoord(worldX), worldToTileCoord(worldZ))
    )
  }

  hasHeightDataForGrid(worldX: number, worldZ: number): boolean {
    const floorX = Math.floor(worldX / TERRAIN_TILE_SIZE)
    const floorZ = Math.floor(worldZ / TERRAIN_TILE_SIZE)
    for (let dz = 0; dz <= 1; dz++) {
      for (let dx = 0; dx <= 1; dx++) {
        if (!this.state.heightmaps.has(tileKey(floorX + dx, floorZ + dz))) {
          return false
        }
      }
    }
    return true
  }

  getHeightAtWorldPosition(worldX: number, worldZ: number): number {
    const tileX = worldToTileCoord(worldX)
    const tileZ = worldToTileCoord(worldZ)
    const tileMinX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
    const tileMinZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
    const localX = worldX - tileMinX
    const localZ = worldZ - tileMinZ
    const cellX = Math.floor(localX)
    const cellZ = Math.floor(localZ)
    const fracX = localX - cellX
    const fracZ = localZ - cellZ

    const h00 = getHeightAtCell(this.state, tileX, tileZ, cellX, cellZ)
    const h10 = getHeightAtCell(this.state, tileX, tileZ, cellX + 1, cellZ)
    const h01 = getHeightAtCell(this.state, tileX, tileZ, cellX, cellZ + 1)
    const h11 = getHeightAtCell(this.state, tileX, tileZ, cellX + 1, cellZ + 1)

    const h0 = h00 + (h10 - h00) * fracX
    const h1 = h01 + (h11 - h01) * fracX
    return h0 + (h1 - h0) * fracZ
  }

  hasWater(tileX: number, tileZ: number): boolean {
    const data = this.state.heightmaps.get(tileKey(tileX, tileZ))
    if (!data) return false
    for (let i = 0; i < data.length; i++) {
      if (data[i] < SEA_LEVEL_ENCODED) return true
    }
    return false
  }

  // --- Geometry ---

  registerGeometry(
    tileX: number,
    tileZ: number,
    geometry: THREE.BufferGeometry
  ) {
    this.state.geometries.set(tileKey(tileX, tileZ), geometry)
  }

  unregisterGeometry(tileX: number, tileZ: number) {
    this.state.geometries.delete(tileKey(tileX, tileZ))
  }

  applyHeightToGeometry(
    tileX: number,
    tileZ: number,
    geometry: THREE.BufferGeometry
  ) {
    applyHeightToGeo(this.state, tileX, tileZ, geometry)
  }

  refreshTileGeometry(tileX: number, tileZ: number): void {
    const key = tileKey(tileX, tileZ)
    const geo = this.state.geometries.get(key)
    if (geo && this.state.heightmaps.has(key)) {
      applyHeightToGeo(this.state, tileX, tileZ, geo)
    }
  }

  refreshAdjacentTileEdges(tileX: number, tileZ: number): void {
    doRefreshAdjacentEdges(this.state, tileX, tileZ)
  }

  getHeightmapTexture(tileX: number, tileZ: number): THREE.DataTexture | null {
    const data = this.state.heightmaps.get(tileKey(tileX, tileZ))
    if (!data) return null

    const W = VERTS_PER_SIDE
    const decoded = new Float32Array(W * W)
    for (let i = 0; i < W * W; i++) {
      decoded[i] = decodeHeight(data[i])
    }

    const tex = new THREE.DataTexture(
      decoded,
      W,
      W,
      THREE.RedFormat,
      THREE.FloatType
    )
    tex.flipY = true
    tex.minFilter = THREE.LinearFilter
    tex.magFilter = THREE.LinearFilter
    tex.needsUpdate = true
    return tex
  }

  updateHeightmapTexture(
    tileX: number,
    tileZ: number,
    tex: THREE.DataTexture
  ): boolean {
    const data = this.state.heightmaps.get(tileKey(tileX, tileZ))
    if (!data) return false

    const W = VERTS_PER_SIDE
    const buf = tex.image.data as Float32Array
    for (let i = 0; i < W * W; i++) {
      buf[i] = decodeHeight(data[i])
    }
    tex.needsUpdate = true
    return true
  }

  // --- Brush operations ---

  applyBrush(
    worldX: number,
    worldZ: number,
    radius: number,
    strengthPerSec: number,
    raise: boolean,
    deltaTimeSec: number,
    isProtected?: (worldX: number, worldZ: number) => boolean
  ): AffectedTile[] {
    const affected = doBrush(
      this.state,
      worldX,
      worldZ,
      radius,
      strengthPerSec,
      raise,
      deltaTimeSec,
      isProtected
    )
    this.finalize(affected)
    return affected
  }

  applyFlatten(
    worldX: number,
    worldZ: number,
    radius: number,
    isProtected?: (worldX: number, worldZ: number) => boolean
  ): AffectedTile[] {
    const affected = doFlatten(this.state, worldX, worldZ, radius, isProtected)
    this.finalize(affected)
    return affected
  }

  applyFlattenLine(
    x1: number,
    z1: number,
    x2: number,
    z2: number,
    radius: number,
    isProtected?: (worldX: number, worldZ: number) => boolean
  ): AffectedTile[] {
    const affected = doFlattenLine(
      this.state,
      x1,
      z1,
      x2,
      z2,
      radius,
      isProtected
    )
    this.finalize(affected)
    return affected
  }

  flattenArea(
    minX: number,
    minZ: number,
    maxX: number,
    maxZ: number,
    targetHeight: number,
    blendRadius: number,
    isProtected?: (worldX: number, worldZ: number) => boolean
  ): AffectedTile[] {
    const affected = doFlattenArea(
      this.state,
      minX,
      minZ,
      maxX,
      maxZ,
      targetHeight,
      blendRadius,
      (tx, tz) => this.ensureOriginalHeightmap(tx, tz),
      isProtected
    )
    this.finalize(affected)
    return affected
  }

  flattenRotatedRect(
    centerX: number,
    centerZ: number,
    rotationDeg: number,
    localMinX: number,
    localMaxX: number,
    localMinZ: number,
    localMaxZ: number,
    targetHeight: number,
    blendRadius: number,
    isProtected?: (worldX: number, worldZ: number) => boolean
  ): AffectedTile[] {
    const affected = doFlattenRotatedRect(
      this.state,
      centerX,
      centerZ,
      rotationDeg,
      localMinX,
      localMaxX,
      localMinZ,
      localMaxZ,
      targetHeight,
      blendRadius,
      (tx, tz) => this.ensureOriginalHeightmap(tx, tz),
      isProtected
    )
    this.finalize(affected)
    return affected
  }

  restoreFromOriginal(
    minX: number,
    minZ: number,
    maxX: number,
    maxZ: number
  ): AffectedTile[] {
    const affected = doRestore(this.state, minX, minZ, maxX, maxZ)
    this.finalize(affected)
    return affected
  }

  // --- Data management ---

  setHeightmap(tileX: number, tileZ: number, data: Uint16Array): void {
    this.state.heightmaps.set(tileKey(tileX, tileZ), data)
  }

  markDirty(tileX: number, tileZ: number): void {
    this.state.dirtyTiles.add(tileKey(tileX, tileZ))
  }

  async saveAllDirty(): Promise<void> {
    if (this.saveTimer !== null) {
      clearTimeout(this.saveTimer)
      this.saveTimer = null
    }
    await saveDirtyTiles(this.state, this.terrainApiUrl)
  }

  unloadTile(tileX: number, tileZ: number) {
    const key = tileKey(tileX, tileZ)
    this.state.heightmaps.delete(key)
    this.state.originalHeightmaps.delete(key)
    this.state.geometries.delete(key)
  }

  evictCachedData(tileX: number, tileZ: number) {
    const key = tileKey(tileX, tileZ)
    if (this.state.dirtyTiles.has(key)) return
    this.state.heightmaps.delete(key)
    if (!this.state.dirtyOriginalTiles.has(key)) {
      this.state.originalHeightmaps.delete(key)
    }
  }

  async destroy() {
    if (this.saveTimer !== null) {
      clearTimeout(this.saveTimer)
      this.saveTimer = null
    }
    if (
      this.state.dirtyTiles.size > 0 ||
      this.state.dirtyOriginalTiles.size > 0
    ) {
      await saveDirtyTiles(this.state, this.terrainApiUrl)
    }
  }
}

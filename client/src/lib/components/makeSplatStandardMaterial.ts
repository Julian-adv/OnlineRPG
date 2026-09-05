// makeSplatStandardMaterial.ts — TSL/WebGPU, V2 palette-based splatmap
// Per-cell encoding (see doc/SPLATMAP_V2.md):
//   byte 0: (primaryIdx << 4) | secondaryIdx  (each 0..15)
//   byte 1: reserved
//   byte 2: blend (0 = 100% primary, 255 = 100% secondary)
//   byte 3: vegMeta (grass density / subtype, read by grass system, not shader)
import * as THREE from 'three'
import { MeshStandardNodeMaterial } from 'three/webgpu'
import {
  Fn,
  Discard,
  uniform,
  uniformArray,
  texture,
  uv,
  vec2,
  vec3,
  vec4,
  float,
  int,
  smoothstep,
  mix,
  min,
  max,
  varying,
  positionLocal,
  modelWorldMatrix,
  fwidth,
  fract,
  abs,
  distance,
  dFdx,
  dFdy,
  TBNViewMatrix,
} from 'three/tsl'
import type { Node, TextureNode, UniformNode } from 'three/webgpu'
import {
  ATLAS_BORDER,
  ATLAS_GRID,
  ATLAS_SLOT_SIZE,
  type SplatAtlasSet,
} from '../utils/splatLayerLoader'
import { MAX_PALETTE } from '../terrain/splat-encoding'
import {
  LAND_PLOT_SIZE,
  REGION_CELLS,
  SPLAT_PADDED_DIM,
  TILE_DIM,
} from '../terrain/terrain-constants'

export type SplatLayer = {
  map: THREE.Texture // Albedo (sRGB)
  normalMap?: THREE.Texture // Normal (Linear)
  orm?: THREE.Texture // ORM: R=AO, G=Roughness, B=Metallic (Linear)
  tile: number
  /** Swap U↔V on this slot (perceptual 90° rotation for isotropic textures). */
  swapUv: boolean
}

export type SplatParams = {
  atlas: SplatAtlasSet
  /** Tile scales for each palette slot. Length 1..MAX_PALETTE; padded to MAX_PALETTE internally. */
  tileScales: number[]
  /** Per-slot U↔V swap flags. Length 1..MAX_PALETTE; padded to 0. */
  tileSwapUvs?: boolean[]
  splatMap: THREE.Texture
  splatScale?: number
  sharedBrushUniforms?: SplatBrushUniforms
  /** Shared hole uniforms — discards fragments inside a world-space rect
   *  (e.g. dungeon entrances). When omitted, the material gets its own
   *  inactive instance. */
  sharedHoleUniforms?: SplatHoleUniforms
  /** Include grid/brush editor overlay in the shader. Default false. */
  includeEditorOverlay?: boolean
}

export interface SplatBrushUniforms {
  brushCenter: UniformNode<'vec2', THREE.Vector2>
  brushRadius: UniformNode<'float', number>
  brushActive: UniformNode<'float', number>
  brushRaise: UniformNode<'float', number>
  brushToolMode: UniformNode<'float', number>
  gridVisible: UniformNode<'float', number>
}

export function createSplatBrushUniforms(): SplatBrushUniforms {
  return {
    brushCenter: uniform(new THREE.Vector2(0, 0)),
    brushRadius: uniform(3.0),
    brushActive: uniform(0.0),
    brushRaise: uniform(1.0),
    brushToolMode: uniform(0.0),
    gridVisible: uniform(0.0),
  }
}

/** Shared uniforms that punch a world-space rectangular hole in the terrain
 *  (fragments inside the rect are discarded). Used to open up dungeon
 *  entrances so the descending stairs aren't occluded by ground. */
export interface SplatHoleUniforms {
  /** (minX, minZ, maxX, maxZ) in world space. */
  holeRect: UniformNode<'vec4', THREE.Vector4>
  /** 0 = no hole, 1 = active. */
  holeActive: UniformNode<'float', number>
}

export function createSplatHoleUniforms(): SplatHoleUniforms {
  return {
    holeRect: uniform(new THREE.Vector4(0, 0, 0, 0)),
    holeActive: uniform(0.0),
  }
}

/** Pad a tileScales array to length MAX_PALETTE with 1.0. Returns a new array. */
export function padTileScales(tileScales: number[]): number[] {
  const out = new Array<number>(MAX_PALETTE).fill(1)
  const n = Math.min(tileScales.length, MAX_PALETTE)
  for (let i = 0; i < n; i++) out[i] = tileScales[i]
  return out
}

/** Pad a boolean swap-flag array to length MAX_PALETTE with false, encoding
 *  each flag as 0.0/1.0 so it can live in a float uniform array and be
 *  multiplied with UV components without a shader branch. */
export function padTileSwapUvs(tileSwapUvs: boolean[]): number[] {
  const out = new Array<number>(MAX_PALETTE).fill(0)
  const n = Math.min(tileSwapUvs.length, MAX_PALETTE)
  for (let i = 0; i < n; i++) out[i] = tileSwapUvs[i] ? 1 : 0
  return out
}

// Atlas slot geometry in normalized UV space.
const SLOT_PX = ATLAS_SLOT_SIZE + 2 * ATLAS_BORDER
const ATLAS_PX = SLOT_PX * ATLAS_GRID
const SUBTEX_NORM = ATLAS_SLOT_SIZE / ATLAS_PX
const BORDER_NORM = ATLAS_BORDER / ATLAS_PX
const GRID_INV = 1.0 / ATLAS_GRID

export function makeSplatStandardMaterial({
  atlas,
  tileScales,
  tileSwapUvs = [],
  splatMap,
  splatScale = 1,
  sharedBrushUniforms,
  sharedHoleUniforms,
  includeEditorOverlay = false,
}: SplatParams) {
  // Splat bytes are integer indices — must NOT be bilinearly interpolated.
  splatMap.minFilter = THREE.NearestFilter
  splatMap.magFilter = THREE.NearestFilter
  splatMap.generateMipmaps = false
  splatMap.anisotropy = 1
  splatMap.needsUpdate = true

  const uTileScales = uniformArray<'float'>(padTileScales(tileScales), 'float')
  const uTileSwapUvs = uniformArray<'float'>(
    padTileSwapUvs(tileSwapUvs),
    'float'
  )
  const uSplatScale = uniform(splatScale)

  // Always compiled in (single pipeline variant). Inactive by default, so
  // the discard is a no-op for the vast majority of fragments.
  const hole = sharedHoleUniforms ?? createSplatHoleUniforms()

  const brush = includeEditorOverlay
    ? {
        center:
          sharedBrushUniforms?.brushCenter ?? uniform(new THREE.Vector2(0, 0)),
        radius: sharedBrushUniforms?.brushRadius ?? uniform(3.0),
        active: sharedBrushUniforms?.brushActive ?? uniform(0.0),
        raise: sharedBrushUniforms?.brushRaise ?? uniform(1.0),
        toolMode: sharedBrushUniforms?.brushToolMode ?? uniform(0.0),
        gridVisible: sharedBrushUniforms?.gridVisible ?? uniform(0.0),
      }
    : null

  const splatTex = texture(splatMap)
  const diffAtlasTex = texture(atlas.diffuseAtlas)
  const normAtlasTex = atlas.normalAtlas ? texture(atlas.normalAtlas) : null
  const ormAtlasTex = atlas.ormAtlas ? texture(atlas.ormAtlas) : null

  const vUvSplat = varying(vec2(0), 'v_uvSplat')
  const vWorldXZ = varying(vec2(0), 'v_worldXZ')

  const vertexNode = Fn(() => {
    const localUv = uv()
    vUvSplat.assign(localUv.mul(uSplatScale))
    const worldPos4 = modelWorldMatrix.mul(vec4(positionLocal, 1.0))
    vWorldXZ.assign(worldPos4.xz)
    return positionLocal
  })()

  // Splat pixel = cell corner (grid vertex); see `doc/SPLATMAP_V2.md`
  // §6. Bilerp-of-resolved-corners smooths palette-pair boundaries (e.g.
  // (SAND,GROUND) ↔ (GROUND,DIRT)) that a nearest-cell approach snapped
  // at a half-cell seam.
  //
  // Texture is SPLAT_PADDED_DIM²; the tile's 64×64 data lives at
  // interior [1..TILE_DIM]. The +1.5 texel shift lands cell 0 on
  // interior pixel 1 instead of padding pixel 0.
  const SPLAT_TEXEL = 1.0 / SPLAT_PADDED_DIM

  const cellPos = vUvSplat.mul(float(TILE_DIM))
  const baseUv = cellPos.floor().add(1.5).mul(SPLAT_TEXEL)
  const fracUv = fract(cellPos)
  const s00 = splatTex.sample(baseUv).toVar()
  const s10 = splatTex.sample(baseUv.add(vec2(SPLAT_TEXEL, 0))).toVar()
  const s01 = splatTex.sample(baseUv.add(vec2(0, SPLAT_TEXEL))).toVar()
  const s11 = splatTex
    .sample(baseUv.add(vec2(SPLAT_TEXEL, SPLAT_TEXEL)))
    .toVar()

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function decodeCell(sample: any) {
    const pk = sample.r.mul(255.0).add(0.5).floor()
    const p = pk.div(16.0).floor().toVar()
    const s = pk.sub(p.mul(16.0)).toVar()
    return { p, s, blend: sample.b }
  }

  const d00 = decodeCell(s00)
  const d10 = decodeCell(s10)
  const d01 = decodeCell(s01)
  const d11 = decodeCell(s11)

  const w00 = fracUv.x.oneMinus().mul(fracUv.y.oneMinus())
  const w10 = fracUv.x.mul(fracUv.y.oneMinus())
  const w01 = fracUv.x.oneMinus().mul(fracUv.y)
  const w11 = fracUv.x.mul(fracUv.y)

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function bilerp4(v00: any, v10: any, v01: any, v11: any) {
    return v00.mul(w00).add(v10.mul(w10)).add(v01.mul(w01)).add(v11.mul(w11))
  }

  const fLocalUv = uv()
  const fUvDx = dFdx(fLocalUv)
  const fUvDy = dFdy(fLocalUv)

  // Compute atlas UV + texture gradients for a given slot index.
  // idxF: float 0..MAX_PALETTE-1. swap: 0 or 1; when 1, rotates the slot's
  // UV 90° CW `(u, v) → (v, -u)`. A transpose would look similar but flips
  // handedness — preserve rotation so tangent-space normals stay consistent.
  // `swap` is echoed back so the normal path can apply the matching rotation.
  function slotUv(
    idxF: Node<'float'>,
    tileScale: Node<'float'>,
    swap: Node<'float'>
  ) {
    const slotCol = idxF.mod(float(ATLAS_GRID))
    const slotRow = idxF.div(float(ATLAS_GRID)).floor()
    const slotOffset = vec2(slotCol, slotRow).mul(GRID_INV)
    const tiled = fLocalUv.mul(tileScale)
    const tiledRot = vec2(tiled.y, tiled.x.negate())
    const uvLocal = mix(tiled, tiledRot, swap)
    const atlasUv = fract(uvLocal)
      .mul(SUBTEX_NORM)
      .add(slotOffset)
      .add(BORDER_NORM)
    const gxRaw = fUvDx.mul(tileScale)
    const gyRaw = fUvDy.mul(tileScale)
    const gx = mix(gxRaw, vec2(gxRaw.y, gxRaw.x.negate()), swap).mul(
      SUBTEX_NORM
    )
    const gy = mix(gyRaw, vec2(gyRaw.y, gyRaw.x.negate()), swap).mul(
      SUBTEX_NORM
    )
    return { atlasUv, gx, gy, swap }
  }

  // Per-neighbor atlas slots + blend. colorNode/normalNode/orm each
  // sample both slots per neighbor and bilerp the 4 resolved values.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function neighborSlots(d: any) {
    const tP = uTileScales.element(int(d.p)).toVar()
    const tS = uTileScales.element(int(d.s)).toVar()
    const swapP = uTileSwapUvs.element(int(d.p)).toVar()
    const swapS = uTileSwapUvs.element(int(d.s)).toVar()
    return {
      pSlot: slotUv(d.p, tP, swapP),
      sSlot: slotUv(d.s, tS, swapS),
      blend: d.blend,
    }
  }

  const n00 = neighborSlots(d00)
  const n10 = neighborSlots(d10)
  const n01 = neighborSlots(d01)
  const n11 = neighborSlots(d11)

  function sampleAtlasAt(
    atlasTex: TextureNode,
    slot: ReturnType<typeof slotUv>
  ) {
    return (atlasTex.sample(slot.atlasUv) as unknown as TextureNode).grad(
      slot.gx,
      slot.gy
    )
  }

  // ─── Color node ─────────────────────────────────────────
  const colorNode = Fn(() => {
    // Punch a world-space hole (e.g. a dungeon entrance) so the descending
    // stairs aren't occluded by terrain. World-space test → applies on
    // whichever tile covers the entrance, no per-tile geometry changes.
    Discard(
      hole.holeActive
        .greaterThan(0.5)
        .and(vWorldXZ.x.greaterThanEqual(hole.holeRect.x))
        .and(vWorldXZ.x.lessThanEqual(hole.holeRect.z))
        .and(vWorldXZ.y.greaterThanEqual(hole.holeRect.y))
        .and(vWorldXZ.y.lessThanEqual(hole.holeRect.w))
    )

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    function neighborDiffuse(n: any) {
      const cP = sampleAtlasAt(diffAtlasTex, n.pSlot).rgb
      const cS = sampleAtlasAt(diffAtlasTex, n.sSlot).rgb
      return mix(cP, cS, n.blend)
    }
    const blended = bilerp4(
      neighborDiffuse(n00),
      neighborDiffuse(n10),
      neighborDiffuse(n01),
      neighborDiffuse(n11)
    )

    if (!brush) return vec4(blended, 1.0)

    const b = blended.toVar()
    const gridActive = smoothstep(float(0.49), float(0.51), brush.gridVisible)

    // Antialiased 1-px line where coords cross an integer
    const gridLine = (coords: Node<'vec2'>) => {
      const g = abs(fract(coords.sub(0.5)).sub(0.5)).div(fwidth(coords))
      return float(1).sub(min(min(g.x, g.y), float(1)))
    }
    const line1 = gridLine(fLocalUv.mul(TILE_DIM))
    const line64 = gridLine(fLocalUv)
    const linePlot = gridLine(vWorldXZ.div(LAND_PLOT_SIZE))
    const lineRegion = gridLine(vWorldXZ.add(TILE_DIM / 2).div(REGION_CELLS))

    b.assign(mix(b, vec3(0, 0, 0), line1.mul(0.3).mul(gridActive)))
    b.assign(mix(b, vec3(1, 1, 1), linePlot.mul(0.7).mul(gridActive)))
    b.assign(mix(b, vec3(1, 0, 0), line64.mul(gridActive)))
    b.assign(mix(b, vec3(0.886, 0.725, 0.231), lineRegion.mul(gridActive)))

    const bDist = distance(vWorldXZ, vec2(brush.center))
    const ringWidth = max(float(0.5), float(brush.radius).mul(0.1))
    const innerRadius = float(brush.radius).sub(ringWidth)
    const inRing = smoothstep(innerRadius.sub(0.1), innerRadius, bDist).mul(
      float(1).sub(
        smoothstep(float(brush.radius), float(brush.radius).add(0.1), bDist)
      )
    )
    const heightColor = mix(
      vec3(1.0, 0.3, 0.3),
      mix(
        vec3(0.3, 1.0, 0.3),
        vec3(0.3, 0.6, 1.0),
        smoothstep(float(1.49), float(1.51), brush.raise)
      ),
      smoothstep(float(0.49), float(0.51), brush.raise)
    )
    const brushColor = mix(
      heightColor,
      vec3(1.0, 0.7, 0.2),
      smoothstep(float(0.49), float(0.51), brush.toolMode)
    )
    const brushAlpha = inRing
      .mul(0.35)
      .mul(smoothstep(float(0.49), float(0.51), brush.active))
    b.assign(mix(b, brushColor, brushAlpha))

    return vec4(b, 1.0)
  })()

  // ─── Normal node ────────────────────────────────────────
  const tbn = TBNViewMatrix as unknown as Node<'mat3'>
  const normalNode = normAtlasTex
    ? Fn(() => {
        const normTex = normAtlasTex!
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        function rotateNormalXy(n: any, swap: any) {
          // Match slotUv's 90° CW rotation in tangent space:
          // (nx, ny, nz) → (-ny, nx, nz).
          const rotatedXy = vec2(n.y.negate(), n.x)
          const xy = mix(vec2(n.x, n.y), rotatedXy, swap)
          return vec3(xy.x, xy.y, n.z)
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        function neighborTangentNormal(n: any) {
          const nP = sampleAtlasAt(normTex, n.pSlot).xyz.mul(2.0).sub(1.0)
          const nS = sampleAtlasAt(normTex, n.sSlot).xyz.mul(2.0).sub(1.0)
          return mix(
            rotateNormalXy(nP, n.pSlot.swap),
            rotateNormalXy(nS, n.sSlot.swap),
            n.blend
          )
        }
        const tangentNormal = bilerp4(
          neighborTangentNormal(n00),
          neighborTangentNormal(n10),
          neighborTangentNormal(n01),
          neighborTangentNormal(n11)
        ).normalize()
        return (tbn.mul(tangentNormal) as unknown as Node<'vec3'>).normalize()
      })()
    : undefined

  // ─── ORM node ───────────────────────────────────────────
  const ormBlended = ormAtlasTex
    ? Fn(() => {
        const ormTex = ormAtlasTex!
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        function neighborOrm(n: any) {
          const oP = sampleAtlasAt(ormTex, n.pSlot).rgb
          const oS = sampleAtlasAt(ormTex, n.sSlot).rgb
          return mix(oP, oS, n.blend)
        }
        return bilerp4(
          neighborOrm(n00),
          neighborOrm(n10),
          neighborOrm(n01),
          neighborOrm(n11)
        )
      })()
    : null
  const roughnessNode = ormBlended ? ormBlended.g : undefined
  const metalnessNode = ormBlended ? ormBlended.b : undefined
  const aoNode = ormBlended ? ormBlended.r : undefined

  const mat = new MeshStandardNodeMaterial()
  mat.roughness = 1.0
  mat.metalness = 0.0
  mat.envMapIntensity = 0

  mat.positionNode = vertexNode
  mat.colorNode = colorNode
  if (normalNode) mat.normalNode = normalNode
  if (roughnessNode) mat.roughnessNode = roughnessNode
  if (metalnessNode) mat.metalnessNode = metalnessNode
  if (aoNode) mat.aoNode = aoNode

  mat.userData.uniforms = {
    splatMap: splatTex,
    diffuseAtlas: diffAtlasTex,
    ...(normAtlasTex ? { normalAtlas: normAtlasTex } : {}),
    ...(ormAtlasTex ? { ormAtlas: ormAtlasTex } : {}),
    uTileScales,
    uTileSwapUvs,
    holeRect: hole.holeRect,
    holeActive: hole.holeActive,
    ...(brush
      ? {
          brushCenter: brush.center,
          brushRadius: brush.radius,
          brushActive: brush.active,
          brushRaise: brush.raise,
          brushToolMode: brush.toolMode,
          gridVisible: brush.gridVisible,
        }
      : {}),
  }

  return mat
}

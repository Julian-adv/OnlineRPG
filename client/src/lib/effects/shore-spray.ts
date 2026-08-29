import * as THREE from 'three'
import { MeshBasicNodeMaterial } from 'three/webgpu'
import { assetUrl } from '../utils/assetUrl'
import {
  attribute,
  texture,
  uniform,
  uv,
  vec3,
  float,
  smoothstep as tslSmoothstep,
  length,
} from 'three/tsl'
import {
  SHORE_WAVE_SPEED,
  MOVE_END,
  BRK_START_MOVE,
  BRK_END_MOVE,
} from '../shaders/shore-wave-timing'
import {
  SWELL_SPAWN_DEPTH,
  SWELL_SHORE_DEPTH,
} from '../shaders/water-field-material'
import {
  WATER_FIELD_GRID,
  type WaterFieldTileData,
} from '../utils/water-field-data'
import {
  TERRAIN_TILE_SIZE,
  SEA_LEVEL,
} from '../components/game-scene/terrain-utils'

/**
 * EXPERIMENTAL: whitewater spray thrown up off the breaking-wave crest.
 *
 * The shore swell/break is a pure GPU shader effect (water-field-material),
 * so there is no CPU crest object to attach particles to. This module
 * reconstructs the crest CPU-side and follows it:
 *  - WHERE: per tile we extract every shore cell across the crest's depth
 *    travel (`computeShoreCells`), tagged with its own water depth.
 *  - WHEN/WHICH: each frame the crest sits at depth `center = mix(SPAWN,
 *    SHORE, move)` for each of the two half-offset phases; a cell emits in
 *    proportion to how close its depth is to a currently-breaking crest.
 * So as `center` sweeps shoreward the emitting band sweeps with it — the
 * spray line travels up the beach with the wave.
 *
 * High particle counts: slot allocation is a ring buffer (O(1)) and the
 * billboard matrices are written straight into the instance buffer reusing
 * one camera-facing basis, so tens of thousands of droplets stay affordable.
 */

// ── Tuning (tweak by eye) ───────────────────────────────────
/** Pool size / hard cap on live droplets. Dial down if it costs FPS. */
const MAX_SHORE_SPRAY = 16000
/** Cells farther than this from the player don't emit. */
export const SHORE_SPRAY_EMIT_RADIUS_M = 30
/** Safety cap on cells considered per frame. */
export const SHORE_SPRAY_MAX_ACTIVE = 4000
/** Spawns/sec per cell sitting exactly on a full-strength breaking crest. */
const SHORE_SPRAY_BASE_RATE = 800
/** Depth half-window (m) around the crest a cell still sprays through — the
 *  thickness of the traveling foam band. Thin = a single crisp line. */
const CREST_BAND_M = 0.06
/** Fine nudge (in noisyD space) off the matched crest: positive = shoreward
 *  (toward the swash), negative = seaward (toward the foam). ~0 sits on the
 *  crest since matching is now noisyD-aligned. */
const SPRAY_CREST_DEPTH_BIAS_M = -0.01
/** Shore cells are kept for depths in this range (the crest's break→swash
 *  travel, ≈ mix(SPAWN,SHORE,[0.35..0.9])), plus margin. */
const CELL_DEPTH_MIN = 0.05
const CELL_DEPTH_MAX = 1.1
/** Grid stride for cell extraction (m). 1 = every heightmap texel. */
const CONTOUR_STEP = 1
/** Sea-only gates (match the shader's seaFxGate intent). */
const SPRAY_RIVERNESS_MAX = 0.3
const SPRAY_FLOWMAG_MAX = 0.2
/** Minimum bed slope to trust a shoreward direction. */
const MIN_BED_SLOPE = 0.01

/** Droplet ballistics — mostly a shoreward drift with only a small lift.
 *  Vertical and shoreward speeds are independent so the lift can be tiny
 *  without killing the toward-land motion. */
const SPRAY_GRAVITY = 0.5
const SPRAY_VY_MIN = 0.05
const SPRAY_VY_RANGE = 0.05
/** Shoreward (toward-land) drift speed — the dominant motion. */
const SPRAY_FORWARD_MIN = 0.1
const SPRAY_FORWARD_RANGE = 0.1
/** Velocity scatter (along-crest) applied to the launch direction. */
const SPRAY_SCATTER = 0.1
/** Isotropic spawn-position jitter (m): fills sub-cell gaps without the
 *  tangent streaking. Note it also thickens the line a touch in the depth
 *  direction, so keep it small. */
const SPRAY_JITTER_M = 0.4
const SPRAY_LIFE_MIN = 0.4
const SPRAY_LIFE_RANGE = 0.5
const SPRAY_SCALE_MIN = 0.1
const SPRAY_SCALE_RANGE = 0.15
const SPRAY_QUAD = 0.28

/** Spray fade-out in crest-travel (`move`) space: `brk` supplies the
 *  ramp-in as the wave breaks; this fades the spray back out through the
 *  early run-up so it doesn't persist all the way up the swash. */
const SPRAY_FADE_MOVE_LO = 0.5
const SPRAY_FADE_MOVE_HI = 0.7

const SPRAY_OPACITY_ATTR = 'aShoreSprayOpacity'
const SPRAY_UV_ATTR = 'aShoreSprayUV'
/** Fraction of the foam texture one droplet shows. Bigger = more ragged
 *  structure per particle (so small particles still read as foam, not dots). */
const SPRAY_FOAM_PATCH = 0.34
/** Global opacity multiplier. */
const SPRAY_MAX_OPACITY = 0.7

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type N = any // TSL node

const clamp01 = (x: number) => (x < 0 ? 0 : x > 1 ? 1 : x)
const smoothstep = (a: number, b: number, x: number) => {
  const t = clamp01((x - a) / (b - a))
  return t * t * (3 - 2 * t)
}
const fract = (x: number) => x - Math.floor(x)

// ── Ragged-crest alignment ──────────────────────────────────
// The shader draws the swell/foam in `noisyD = depth + noise(worldXZ, seed)`
// space (buildNoisyDepth), so the visible crest is a ragged contour, NOT the
// clean depth line. To spawn ON that crest we replicate the exact same noise
// CPU-side: same value-noise.jpg, same octave weights/offsets, same seed.
const NOISE_PERIODS = 64 // must match water-field-material.ts
/** Flip the sampled row to match the texture's flipY. If the ragged line
 *  reads ANTI-correlated with the crest (wiggles the wrong way), toggle. */
const NOISE_FLIP_Y = true

let noisePixels: Float32Array | null = null
let noiseSize = 0
let noiseLoading = false
function ensureShoreNoiseLoaded(): void {
  if (noisePixels || noiseLoading) return
  noiseLoading = true
  fetch(assetUrl('/textures/value-noise.jpg'))
    .then((r) => r.blob())
    .then((b) => createImageBitmap(b))
    .then((img) => {
      const cv = document.createElement('canvas')
      cv.width = img.width
      cv.height = img.height
      const ctx = cv.getContext('2d')
      if (!ctx) throw new Error('no 2d context')
      ctx.drawImage(img, 0, 0)
      const data = ctx.getImageData(0, 0, img.width, img.height).data
      const px = new Float32Array(img.width * img.height)
      for (let k = 0; k < px.length; k++) px[k] = data[k * 4] / 255 // .r
      noiseSize = img.width
      noisePixels = px
    })
    .catch((e) => {
      console.error('shore-spray: value-noise load failed', e)
      noiseLoading = false
    })
}

/** Bilinear, repeat-wrapped sample — mirrors the shader's
 *  `noiseTex.sample(coord / NOISE_PERIODS).r`. Returns 0 until loaded (falls
 *  back to the clean depth contour). */
function sampleNoiseCPU(cx: number, cz: number): number {
  const px = noisePixels
  if (!px) return 0
  const size = noiseSize
  const fx = (cx / NOISE_PERIODS) * size - 0.5
  const fz = ((NOISE_FLIP_Y ? -cz : cz) / NOISE_PERIODS) * size - 0.5
  const x0 = Math.floor(fx)
  const z0 = Math.floor(fz)
  const tx = fx - x0
  const tz = fz - z0
  const wrap = (n: number) => ((n % size) + size) % size
  const x0w = wrap(x0)
  const x1w = wrap(x0 + 1)
  const z0w = wrap(z0)
  const z1w = wrap(z0 + 1)
  const p00 = px[z0w * size + x0w]
  const p10 = px[z0w * size + x1w]
  const p01 = px[z1w * size + x0w]
  const p11 = px[z1w * size + x1w]
  const a = p00 + (p10 - p00) * tx
  const b = p01 + (p11 - p01) * tx
  return a + (b - a) * tz
}

/** CPU replica of the shader's buildNoisyDepth (three octaves, per-wave
 *  seed offsets). */
function noisyDepthCPU(
  depth: number,
  wx: number,
  wz: number,
  seed: number
): number {
  return (
    depth +
    sampleNoiseCPU(wx * 0.3 + seed * 17.13, wz * 0.3 + seed * 29.71) * 0.15 +
    sampleNoiseCPU(wx * 0.15 + seed * 31.37, wz * 0.15 + seed * 11.79) * 0.1 +
    sampleNoiseCPU(wx * 0.2 + seed * 7.43, wz * 0.2 + seed * 23.17) * 0.3
  )
}

/** One breaking phase: the crest's current depth, how hard it's breaking
 *  (0..1), and the per-wave seed. All reconstructed from the shader's shared
 *  constants so the spray tracks the real crest even if those change. */
export interface CrestPhase {
  center: number
  activity: number
  seed: number
}

function crestPhase(waterTime: number, offset: number): CrestPhase {
  const cycle = fract(waterTime * SHORE_WAVE_SPEED + offset)
  const move = smoothstep(0, MOVE_END, cycle)
  const center =
    SWELL_SPAWN_DEPTH + (SWELL_SHORE_DEPTH - SWELL_SPAWN_DEPTH) * move
  const brk = smoothstep(BRK_START_MOVE, BRK_END_MOVE, move)
  const fade = 1 - smoothstep(SPRAY_FADE_MOVE_LO, SPRAY_FADE_MOVE_HI, move)
  // Matches the shader's per-wave `seed` exactly (same decorrelation).
  const seed = fract(
    Math.floor(waterTime * SHORE_WAVE_SPEED + offset) * 0.618034 +
      offset * 0.754877666
  )
  return { center, activity: brk * fade, seed }
}

/** The two half-offset breaking phases at `waterTime`. */
export function computeCrestPhases(waterTime: number): CrestPhase[] {
  return [crestPhase(waterTime, 0), crestPhase(waterTime, 0.5)]
}

export interface ShoreCell {
  x: number
  y: number
  z: number
  /** Water depth here (m) — matched against the crest depth each frame. */
  depth: number
  /** Shoreward (uphill) unit direction — the crest runs perpendicular. */
  shoreX: number
  shoreZ: number
  /** Per-cell spawn accumulator. */
  acc: number
}

/**
 * Extract shore cells for one tile spanning the crest's depth travel,
 * restricted to open sea. `field` is null for tiles with no baked water
 * field (404 = "no river influence") — flat open sea at SEA_LEVEL, the
 * common breaking-wave case — so we synthesize it from the heightmap.
 */
export function computeShoreCells(
  field: WaterFieldTileData | null,
  tileX: number,
  tileZ: number,
  getBedHeight: (x: number, z: number) => number
): ShoreCell[] {
  const originX = tileX * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const originZ = tileZ * TERRAIN_TILE_SIZE - TERRAIN_TILE_SIZE / 2
  const cells: ShoreCell[] = []

  for (let j = 0; j < WATER_FIELD_GRID; j += CONTOUR_STEP) {
    for (let i = 0; i < WATER_FIELD_GRID; i += CONTOUR_STEP) {
      const idx = j * WATER_FIELD_GRID + i
      const riverness = field ? field.riverness[idx] : 0
      const flowMag = field ? Math.hypot(field.flowX[idx], field.flowZ[idx]) : 0
      if (riverness > SPRAY_RIVERNESS_MAX) continue
      if (flowMag > SPRAY_FLOWMAG_MAX) continue

      const wx = originX + i
      const wz = originZ + j
      const surfaceY = field ? field.surfaceY[idx] : SEA_LEVEL
      const bedY = getBedHeight(wx, wz)
      const depth = surfaceY - bedY
      if (depth < CELL_DEPTH_MIN || depth > CELL_DEPTH_MAX) continue

      // Uphill bed gradient = shoreward (matches the shader's landwardDir).
      const dhx = getBedHeight(wx + 1, wz) - getBedHeight(wx - 1, wz)
      const dhz = getBedHeight(wx, wz + 1) - getBedHeight(wx, wz - 1)
      const slope = Math.hypot(dhx, dhz)
      if (slope < MIN_BED_SLOPE) continue

      cells.push({
        x: wx,
        y: surfaceY,
        z: wz,
        depth,
        shoreX: dhx / slope,
        shoreZ: dhz / slope,
        acc: 0,
      })
    }
  }
  return cells
}

interface SprayParticle {
  alive: boolean
  age: number
  maxAge: number
  x: number
  y: number
  z: number
  vx: number
  vy: number
  vz: number
  baseScale: number
}

/**
 * Instanced billboard pool for shore spray. One pool serves every cell; the
 * layer feeds the cells near the player each frame plus the crest phases
 * that gate spawning to breaking waves.
 */
export class ShoreSpraySystem {
  readonly mesh: THREE.InstancedMesh
  private readonly uDayDim = uniform(1)
  private readonly uvAttr: THREE.InstancedBufferAttribute
  private readonly opacityAttr: THREE.InstancedBufferAttribute
  private readonly matArr: Float32Array
  private readonly pool: SprayParticle[] = Array.from(
    { length: MAX_SHORE_SPRAY },
    () => ({
      alive: false,
      age: 0,
      maxAge: 0,
      x: 0,
      y: 0,
      z: 0,
      vx: 0,
      vy: 0,
      vz: 0,
      baseScale: 0,
    })
  )
  /** Ring-buffer write cursor — O(1) slot allocation at high spawn rates. */
  private cursor = 0
  private readonly rotMat = new THREE.Matrix4()

  constructor(foamMap: THREE.Texture) {
    ensureShoreNoiseLoaded()
    const geom = new THREE.PlaneGeometry(SPRAY_QUAD, SPRAY_QUAD)
    this.opacityAttr = new THREE.InstancedBufferAttribute(
      new Float32Array(MAX_SHORE_SPRAY),
      1
    )
    geom.setAttribute(SPRAY_OPACITY_ATTR, this.opacityAttr)
    this.uvAttr = new THREE.InstancedBufferAttribute(
      new Float32Array(MAX_SHORE_SPRAY * 2),
      2
    )
    geom.setAttribute(SPRAY_UV_ATTR, this.uvAttr)

    const mat = new MeshBasicNodeMaterial()
    mat.transparent = true
    mat.depthWrite = false
    mat.side = THREE.DoubleSide
    const foamTex: N = texture(foamMap)
    const quadUV: N = uv()
    const patch = foamTex.sample(
      quadUV.mul(SPRAY_FOAM_PATCH).add(attribute(SPRAY_UV_ATTR, 'vec2'))
    ).r
    // The foam texture defines the ragged silhouette (thresholded so parts
    // stay transparent — a torn foam bit, not a solid disc). A light vignette
    // only clips the quad corners so the texture edge isn't a hard square.
    const vignette = float(1).sub(
      tslSmoothstep(float(0.42), float(0.5), length(quadUV.sub(0.5)))
    )
    mat.colorNode = vec3(0.95, 0.98, 1.0)
    mat.opacityNode = tslSmoothstep(float(0.36), float(0.62), patch)
      .mul(vignette)
      .mul(attribute(SPRAY_OPACITY_ATTR, 'float'))
      .mul(this.uDayDim)
      .mul(SPRAY_MAX_OPACITY)

    this.mesh = new THREE.InstancedMesh(geom, mat, MAX_SHORE_SPRAY)
    this.mesh.frustumCulled = false
    this.mesh.castShadow = false
    this.mesh.receiveShadow = false
    this.mesh.renderOrder = 3
    this.mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage)
    this.opacityAttr.setUsage(THREE.DynamicDrawUsage)
    this.uvAttr.setUsage(THREE.DynamicDrawUsage)
    this.matArr = this.mesh.instanceMatrix.array as Float32Array
    // All slots start as a zero-scale (invisible) matrix — diag(0,0,0,1),
    // so w=1 keeps the degenerate point from producing NaN clip coords.
    this.matArr.fill(0)
    for (let i = 0; i < MAX_SHORE_SPRAY; i++) this.matArr[i * 16 + 15] = 1
  }

  setDayDim(v: number) {
    this.uDayDim.value = v
  }

  update(
    dt: number,
    camera: THREE.Camera,
    cells: ShoreCell[],
    phases: CrestPhase[]
  ) {
    // Spawn along the crest: a cell sprays in proportion to how close its
    // depth is to a currently-breaking crest, so the emitting band travels
    // shoreward as `center` sweeps in.
    for (const c of cells) {
      let rate = 0
      for (const ph of phases) {
        if (ph.activity <= 0) continue
        // Match in the shader's noisyD space so the line rides the ragged
        // crest, not the clean depth contour.
        const noisyD = noisyDepthCPU(c.depth, c.x, c.z, ph.seed)
        const target = ph.center - SPRAY_CREST_DEPTH_BIAS_M
        const w = 1 - smoothstep(0, CREST_BAND_M, Math.abs(noisyD - target))
        if (w > 0) rate += SHORE_SPRAY_BASE_RATE * ph.activity * w
      }
      if (rate < 0.01) {
        c.acc = 0
        continue
      }
      const interval = 1 / rate
      c.acc += dt
      if (c.acc > interval * 3) c.acc = interval * 3 // bound catch-up bursts
      while (c.acc >= interval) {
        c.acc -= interval
        this.spawn(c)
      }
    }

    // Integrate + write billboard matrices directly, reusing one
    // camera-facing basis (no per-particle quaternion/compose).
    this.rotMat.makeRotationFromQuaternion(camera.quaternion)
    const e = this.rotMat.elements
    const arr = this.matArr
    const opac = this.opacityAttr.array as Float32Array
    let alive = 0
    for (let i = 0; i < MAX_SHORE_SPRAY; i++) {
      const p = this.pool[i]
      if (!p.alive) continue
      const o = i * 16
      p.age += dt
      if (p.age >= p.maxAge) {
        p.alive = false
        opac[i] = 0
        // Zero the full 3×3 billboard basis so the quad collapses to a
        // point (zeroing only the diagonal would leave it sheared-visible).
        arr[o] = arr[o + 1] = arr[o + 2] = 0
        arr[o + 4] = arr[o + 5] = arr[o + 6] = 0
        arr[o + 8] = arr[o + 9] = arr[o + 10] = 0
        continue
      }
      alive++
      p.vy -= SPRAY_GRAVITY * dt
      p.x += p.vx * dt
      p.y += p.vy * dt
      p.z += p.vz * dt

      const t = p.age / p.maxAge
      opac[i] = t < 0.15 ? t / 0.15 : t > 0.55 ? 1 - (t - 0.55) / 0.45 : 1
      const s = p.baseScale * (0.7 + t * 0.9)
      arr[o] = e[0] * s
      arr[o + 1] = e[1] * s
      arr[o + 2] = e[2] * s
      arr[o + 3] = 0
      arr[o + 4] = e[4] * s
      arr[o + 5] = e[5] * s
      arr[o + 6] = e[6] * s
      arr[o + 7] = 0
      arr[o + 8] = e[8] * s
      arr[o + 9] = e[9] * s
      arr[o + 10] = e[10] * s
      arr[o + 11] = 0
      arr[o + 12] = p.x
      arr[o + 13] = p.y
      arr[o + 14] = p.z
      arr[o + 15] = 1
    }
    // Skip the buffer re-uploads while idle — no live droplets and nothing
    // spawning. `visible = false` hides any stale (already-zeroed) matrices.
    if (alive > 0 || cells.length > 0) {
      this.mesh.instanceMatrix.needsUpdate = true
      this.opacityAttr.needsUpdate = true
      this.uvAttr.needsUpdate = true
    }
    this.mesh.count = MAX_SHORE_SPRAY
    this.mesh.visible = alive > 0
  }

  private spawn(c: ShoreCell) {
    const slot = this.cursor
    this.cursor = (this.cursor + 1) % MAX_SHORE_SPRAY
    const p = this.pool[slot]
    const uvArr = this.uvAttr.array as Float32Array
    uvArr[slot * 2] = Math.random() * (1 - SPRAY_FOAM_PATCH)
    uvArr[slot * 2 + 1] = Math.random() * (1 - SPRAY_FOAM_PATCH)

    // Fill the sub-cell gaps with a small ISOTROPIC jitter, not a tangent
    // segment: a straight along-crest spread would connect neighbouring
    // cells into long straight streaks and smear the ragged (noisyD) crest
    // back into a smooth line. Random x/z keeps the line ragged + continuous.
    const perpX = -c.shoreZ
    const perpZ = c.shoreX
    p.x = c.x + (Math.random() - 0.5) * 2 * SPRAY_JITTER_M
    p.z = c.z + (Math.random() - 0.5) * 2 * SPRAY_JITTER_M
    p.y = c.y + 0.05

    // Mostly a shoreward (toward-land) drift with only a small lift, plus a
    // little along-crest scatter.
    p.vy = SPRAY_VY_MIN + Math.random() * SPRAY_VY_RANGE
    const shoreward = SPRAY_FORWARD_MIN + Math.random() * SPRAY_FORWARD_RANGE
    p.vx = c.shoreX * shoreward + perpX * (Math.random() - 0.5) * SPRAY_SCATTER
    p.vz = c.shoreZ * shoreward + perpZ * (Math.random() - 0.5) * SPRAY_SCATTER
    p.maxAge = SPRAY_LIFE_MIN + Math.random() * SPRAY_LIFE_RANGE
    p.baseScale = SPRAY_SCALE_MIN + Math.random() * SPRAY_SCALE_RANGE
    p.age = 0
    p.alive = true
  }

  dispose() {
    this.mesh.geometry.dispose()
    if (this.mesh.material instanceof THREE.Material)
      this.mesh.material.dispose()
    this.mesh.removeFromParent()
  }
}

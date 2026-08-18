import * as THREE from 'three'
import { MeshStandardNodeMaterial } from 'three/webgpu'
import { color, float, frontFacing, mix, texture } from 'three/tsl'
import type { WindState } from '../shaders/grass-material'
import {
  BARBARIAN_CHARACTER_MODEL_PATH,
  CAVEMAN_CHARACTER_MODEL_PATH,
  CAVEWOMAN_CHARACTER_MODEL_PATH,
  FEMALE_BARD_CHARACTER_MODEL_PATH,
  FEMALE_BARBARIAN_CHARACTER_MODEL_PATH,
  FEMALE_KNIGHT_CHARACTER_MODEL_PATH,
  FEMALE_PRIEST_CHARACTER_MODEL_PATH,
  FEMALE_ROGUE_CHARACTER_MODEL_PATH,
  PRIEST_CHARACTER_MODEL_PATH,
  ROGUE_CHARACTER_MODEL_PATH,
  VALKYRIE_CHARACTER_MODEL_PATH,
} from '../utils/modelPaths'

/** Procedural back cape: a tapered skinned sheet driven by a tiny verlet cloth
 *  (3 chains × `segments` points). The solve runs in world space, so the sheet
 *  trails its anchor on its own — walking, turning and wind flutter it without
 *  any velocity plumbing. ~18 points per cape. */

/** The wearer's back silhouette in cape space: the smallest z a cloth point may
 *  take at each height below the collar. Keeps the buttocks and shoulder blades
 *  from poking through the sheet. Cape space rides the spine bone, so the
 *  silhouette follows the torso through every pose. */
export interface CapeBody {
  /** Minimum cape-space z, one sample per `step` of drop below the collar. */
  minZ: Float32Array
  step: number
}

export interface CapeRigOptions {
  /** Width at the shoulders (m). */
  topWidth: number
  /** Width at the hem (m). */
  bottomWidth: number
  /** Shoulders-to-hem length (m). */
  length: number
  /** Bones per chain (default 5). */
  segments?: number
  body?: CapeBody
  /** How the cloth looks; defaults to the wool cape's dye, unprinted. */
  skin?: CapeSkin
}

/** What a cape looks like: a dyed colour, optionally printed over. The print
 *  is a URL rather than a texture hash so the picker can try on a local file
 *  through the same path a stored one takes. */
export interface CapeSkin {
  color: THREE.ColorRepresentation
  texture?: string | null
}

/** The part of the world's wind the cloth cares about — a `WindState` from the
 *  grass layer satisfies it as is. */
export type CapeWind = Pick<WindState, 'windDirX' | 'windDirZ' | 'windStrength'>

export interface CapeRig {
  /** Attach to the spine bone. Local frame: +x right, +y up, +z away from back. */
  root: THREE.Group
  mesh: THREE.SkinnedMesh
  update(dt: number, wind: CapeWind | null): void
  /** Re-dye or re-print the hanging cloth. Nothing else in the rig depends on
   *  how it looks, so the pickers swap materials instead of rebuilding the
   *  sheet on every drag of the colour wheel. */
  setSkin(skin: CapeSkin): void
  dispose(): void
}

const COLUMNS = 3
const GEOM_COLS = 7
const DEFAULT_SEGMENTS = 5
const SUBSTEP = 1 / 60
const MAX_SUBSTEPS = 3
/** Heavy cloth: it bleeds speed fast and hangs hard, so a standing start swings
 *  the sheet once instead of setting it flapping. */
const DAMPING = 0.88
const GRAVITY = -14
const CONSTRAINT_ITERATIONS = 3
/** Bend limits: between neighbouring segments, and off the hanging rest pose. */
const MAX_SEGMENT_BEND = Math.PI * 0.28
const MAX_ROOT_BEND = Math.PI * 0.42
/** Gap kept between the sheet and the measured back surface (m). */
const SURFACE_CLEARANCE = 0.02
/** Bows the top corners forward around the shoulders (m). The collar anchors
 *  are kinematic and the profile is the back's deepest point, so this leans on
 *  the shoulders receding from that point by more than the wrap. */
const SHOULDER_WRAP = 0.035
/** Height between back-profile samples (m). */
const PROFILE_STEP = 0.05
/** Fallback surface depth when a rig has no skinned geometry to measure. */
const FALLBACK_BODY_DEPTH = 0.16
/** Drop over which a collar bias fades to nothing, so sinking the collar into
 *  the shoulders never lets the hem through the buttocks. */
const COLLAR_BIAS_FADE = 0.3
export const COLLAR_BIAS_LIMIT = 0.3
/** Cloth colour for a cape whose item def names none. */
export const DEFAULT_CAPE_COLOR = 0x6d1720
/** Width-to-height an uploaded print is stored at. The cloth only ever samples
 *  the middle of a print's width — 0.41 to 0.62 of it, depending on the rig —
 *  so a square file would ship, decode and hold in VRAM a third more pixels
 *  than any wearer can show. 320×512 is the nearest round size above the
 *  widest rig. Changing it makes prints already stored render at the wrong
 *  width: the geometry bakes it in, which is what keeps the crop per-rig and
 *  free of any per-texture state. */
export const PRINT_ASPECT = 320 / 512
const WIND_ACCEL = 2.4
/** Ripple across the sheet: shallow and low-frequency, or the cape reads as a
 *  flag. It grows with travel speed, off a smoothed speed so a standing start
 *  ramps in rather than snapping on. */
const FLUTTER_ACCEL = 1.1
const FLUTTER_FREQ = 4.5
const FLUTTER_FREQ_LATERAL = 3.6
const FLUTTER_SPEED_SCALE = 0.22
const FLUTTER_SPEED_CAP = 1.0
const SPEED_SMOOTHING = 0.35
/** Anchor jump per frame (m) read as a teleport instead of motion. */
const TELEPORT_JUMP = 1.5
/** Rest hang: straight down with a slight backward drape. */
const REST_DIR = new THREE.Vector3(0, -1, 0.22).normalize()

const DOWN = new THREE.Vector3(0, -1, 0)
const UP = new THREE.Vector3(0, 1, 0)

const _d = new THREE.Vector3()
const _delta = new THREE.Vector3()
const _local = new THREE.Vector3()
const _accel = new THREE.Vector3()
const _vel = new THREE.Vector3()
const _next = new THREE.Vector3()
const _prevCenter = new THREE.Vector3()
const _axis = new THREE.Vector3()
const _back = new THREE.Vector3()
const _right = new THREE.Vector3()
const _restWorld = new THREE.Vector3()
const _qInv = new THREE.Quaternion()
const _rootQuat = new THREE.Quaternion()
const _chainQuat = new THREE.Quaternion()
const _rootInv = new THREE.Matrix4()
const _vertex = new THREE.Vector3()

/** Rotate `dir` back toward `reference` when it exceeds `maxAngle`. */
function clampDirection(
  dir: THREE.Vector3,
  reference: THREE.Vector3,
  maxAngle: number
): void {
  const angle = Math.acos(THREE.MathUtils.clamp(dir.dot(reference), -1, 1))
  if (angle <= maxAngle) return
  _axis.crossVectors(reference, dir)
  if (_axis.lengthSq() < 1e-12) return
  _axis.normalize()
  dir.copy(reference).applyAxisAngle(_axis, maxAngle)
}

/** Minimum cape-space z `drop` metres below the collar. Takes the deeper of the
 *  two straddling samples rather than interpolating: lerping across a step in
 *  the silhouette dips below the real surface and lets the body through. */
export function capeMinZAt(body: CapeBody, drop: number): number {
  const t = drop / body.step
  if (t <= 0) return body.minZ[0]
  const last = body.minZ.length - 1
  if (t >= last) return body.minZ[last]
  const i = Math.floor(t)
  return Math.max(body.minZ[i], body.minZ[i + 1])
}

function halfWidthAt(options: CapeRigOptions, v: number): number {
  return THREE.MathUtils.lerp(options.topWidth, options.bottomWidth, v) / 2
}

/** Point on the sheet at (`u` across, `v` down), in cape space. The chain
 *  anchors read the same curve at v = 0, so the collar cannot drift off the
 *  mesh's top edge. */
function sheetPoint(
  options: CapeRigOptions,
  u: number,
  v: number,
  out: THREE.Vector3
): THREE.Vector3 {
  const off = (u - 0.5) * 2
  return out.set(
    off * halfWidthAt(options, v),
    -v * options.length,
    -SHOULDER_WRAP * (1 - v) * off * off
  )
}

/** The two samples straddling `f` and the blend between them, clamped so the
 *  ends of the range weight a single sample. Shared by the skin weights' row
 *  and column math. */
function span(f: number, samples: number): [number, number, number] {
  const i = THREE.MathUtils.clamp(Math.floor(f), 0, samples - 2)
  return [i, i + 1, THREE.MathUtils.clamp(f - i, 0, 1)]
}

function buildGeometry(
  options: CapeRigOptions,
  segments: number
): THREE.BufferGeometry {
  const rows = segments * 2 + 1
  const count = rows * GEOM_COLS
  const positions = new Float32Array(count * 3)
  const uvs = new Float32Array(count * 2)
  // A print stretched across the whole sheet would come out elongated: the
  // sheet is about twice as tall as it is wide, and by a different amount per
  // rig. So the print fills the drop and the cloth shows this much of its
  // width, cropping the sides (doc/CAPE_CUSTOMIZATION.md, "천에 UV 입히기").
  // Measured against the drop, then read back against how prints are stored.
  const printCrop = Math.min(
    PRINT_ASPECT,
    (2 * halfWidthAt(options, 0.5)) / options.length
  )
  const skinIndices = new Uint16Array(count * 4)
  const skinWeights = new Float32Array(count * 4)

  for (let r = 0; r < rows; r++) {
    const v = r / (rows - 1)
    const [i0, i1, rowT] = span(v * segments - 0.5, segments)

    for (let c = 0; c < GEOM_COLS; c++) {
      const idx = r * GEOM_COLS + c
      const u = c / (GEOM_COLS - 1)

      sheetPoint(options, u, v, _vertex).toArray(positions, idx * 3)
      uvs[idx * 2] = ((u - 0.5) * printCrop) / PRINT_ASPECT + 0.5
      uvs[idx * 2 + 1] = 1 - v

      const [c0, c1, colT] = span(u * (COLUMNS - 1), COLUMNS)

      skinIndices[idx * 4] = c0 * segments + i0
      skinIndices[idx * 4 + 1] = c0 * segments + i1
      skinIndices[idx * 4 + 2] = c1 * segments + i0
      skinIndices[idx * 4 + 3] = c1 * segments + i1
      skinWeights[idx * 4] = (1 - colT) * (1 - rowT)
      skinWeights[idx * 4 + 1] = (1 - colT) * rowT
      skinWeights[idx * 4 + 2] = colT * (1 - rowT)
      skinWeights[idx * 4 + 3] = colT * rowT
    }
  }

  const indices: number[] = []
  for (let r = 0; r < rows - 1; r++) {
    for (let c = 0; c < GEOM_COLS - 1; c++) {
      const a = r * GEOM_COLS + c
      const b = a + 1
      const cc = a + GEOM_COLS
      const d = cc + 1
      indices.push(a, cc, b, b, cc, d)
    }
  }

  const geometry = new THREE.BufferGeometry()
  geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))
  geometry.setAttribute('uv', new THREE.BufferAttribute(uvs, 2))
  geometry.setAttribute('skinIndex', new THREE.BufferAttribute(skinIndices, 4))
  geometry.setAttribute('skinWeight', new THREE.BufferAttribute(skinWeights, 4))
  geometry.setIndex(indices)
  geometry.computeVertexNormals()
  // The bind-pose bounds don't cover a sheet that swings, so widen them once
  // here. Without it the mesh has to opt out of frustum culling entirely, and
  // an uncullable cape is drawn again in every shadow pass.
  geometry.boundingSphere = new THREE.Sphere(
    new THREE.Vector3(0, -options.length / 2, 0),
    options.length * 1.6
  )
  return geometry
}

/** Sheets and cloth, shared by everyone they fit. The skeleton stays
 *  per-wearer; geometry and material do not vary with the pose, and both are
 *  bounded by the handful of (model, bias) fits in play — so a crowd costs
 *  one set of GPU buffers, not one per cape. */
const geometryCache = new Map<string, THREE.BufferGeometry>()

/** Cape colours are dyed per player (doc/CAPE_CUSTOMIZATION.md), so unlike
 *  the fits they have no small fixed set to converge on — a cache that only
 *  ever grew would hold one material per colour ever seen. Counting wearers
 *  shares one material across a crowd in the same skin and bounds the cache
 *  at exactly what is on screen. */
const materialCache = new Map<
  string,
  { material: THREE.Material; refs: number }
>()

/** Prints are keyed by URL alone, one level below the materials: the same
 *  picture over two different dyes is two materials but one decode and one
 *  1MB GPU texture. Refcounted rather than kept forever like the icon cache,
 *  because player uploads are unbounded. */
const printCache = new Map<string, { map: THREE.Texture; refs: number }>()

const printLoader = new THREE.TextureLoader()

/** A rig's hold on one shared material, and the skin that names it — the skin
 *  is kept so releasing knows which print to let go of too. */
interface Claimed {
  key: string
  skin: CapeSkin
  material: THREE.Material
}

function skinKey(skin: CapeSkin): string {
  return `${skin.color}|${skin.texture ?? ''}`
}

function sharedGeometry(
  options: CapeRigOptions,
  segments: number
): THREE.BufferGeometry {
  const key = `${options.topWidth}|${options.bottomWidth}|${options.length}|${segments}`
  let geometry = geometryCache.get(key)
  if (!geometry) {
    geometry = buildGeometry(options, segments)
    geometryCache.set(key, geometry)
  }
  return geometry
}

/** Claim the material for `skin`, counting this rig as a wearer. Every claim
 *  is paired with a `releaseMaterial`. */
function claimMaterial(skin: CapeSkin): Claimed {
  const key = skinKey(skin)
  let entry = materialCache.get(key)
  if (!entry) {
    entry = {
      material: new MeshStandardNodeMaterial({
        color: new THREE.Color(skin.color),
        roughness: 0.74,
        metalness: 0,
        side: THREE.DoubleSide,
      }),
      refs: 0,
    }
    materialCache.set(key, entry)
    if (skin.texture) claimPrint(key, skin)
  }
  entry.refs++
  return { key, skin, material: entry.material }
}

/** Fetch an uploaded print and lay it over the dye once it arrives. A hash an
 *  admin has blocked stops being served, so the load simply fails and the cape
 *  keeps wearing its colour — no sweep needed to unwear a blocked picture. */
function claimPrint(key: string, skin: CapeSkin) {
  const url = skin.texture
  if (!url) return
  const cached = printCache.get(url)
  if (cached) {
    cached.refs++
    paintPrint(key, skin, cached.map)
    return
  }
  printLoader.load(
    url,
    (map) => {
      // The wearer may have taken it off while this was in flight; then
      // nothing holds the print and it must not enter the cache.
      if (!materialCache.has(key)) {
        map.dispose()
        return
      }
      map.colorSpace = THREE.SRGBColorSpace
      printCache.set(url, { map, refs: 1 })
      paintPrint(key, skin, map)
    },
    undefined,
    // A blocked or missing print is the expected way a cape loses one, not a
    // fault worth a console line every time it comes into view.
    () => {}
  )
}

function paintPrint(key: string, skin: CapeSkin, map: THREE.Texture) {
  const entry = materialCache.get(key)
  if (!entry) return
  const print = texture(map)
  // Opaque still: the mix happens in the shader, so there is no sorting or
  // depth cost to pay. The sheet is DoubleSide and the print would read
  // mirrored from behind, so the lining keeps the dye.
  const material = entry.material as MeshStandardNodeMaterial
  material.colorNode = mix(
    color(new THREE.Color(skin.color)),
    print.rgb,
    print.a.mul(float(frontFacing))
  )
  material.needsUpdate = true
}

function releasePrint(url: string) {
  const entry = printCache.get(url)
  if (!entry || --entry.refs > 0) return
  entry.map.dispose()
  printCache.delete(url)
}

function releaseMaterial(claimed: Claimed) {
  const entry = materialCache.get(claimed.key)
  if (!entry || --entry.refs > 0) return
  entry.material.dispose()
  materialCache.delete(claimed.key)
  if (claimed.skin.texture) releasePrint(claimed.skin.texture)
}

export function createCapeRig(options: CapeRigOptions): CapeRig {
  const segments = options.segments ?? DEFAULT_SEGMENTS
  const segLen = options.length / segments
  const body = options.body ?? null

  const root = new THREE.Group()
  const geometry = sharedGeometry(options, segments)
  let claimed = claimMaterial(options.skin ?? { color: DEFAULT_CAPE_COLOR })

  const mesh = new THREE.SkinnedMesh(geometry, claimed.material)
  mesh.castShadow = true
  mesh.receiveShadow = true
  root.add(mesh)

  // Collar anchors, fixed in cape space: the sheet's top edge at each chain.
  const chainTops = Array.from({ length: COLUMNS }, (_, c) =>
    sheetPoint(options, c / (COLUMNS - 1), 0, new THREE.Vector3())
  )

  const bones: THREE.Bone[] = []
  for (let c = 0; c < COLUMNS; c++) {
    let parent: THREE.Object3D = mesh
    for (let i = 0; i < segments; i++) {
      const bone = new THREE.Bone()
      if (i === 0) bone.position.copy(chainTops[c])
      else bone.position.set(0, -segLen, 0)
      parent.add(bone)
      parent = bone
      bones.push(bone)
    }
  }

  const skeleton = new THREE.Skeleton(bones)
  root.updateMatrixWorld(true)
  mesh.bind(skeleton)

  // Simulation state: [chain][point], point 0 is the kinematic anchor.
  const points: THREE.Vector3[][] = []
  const prev: THREE.Vector3[][] = []
  const anchors: THREE.Vector3[] = []
  const chainDirs: THREE.Vector3[] = []
  for (let c = 0; c < COLUMNS; c++) {
    points.push(Array.from({ length: segments + 1 }, () => new THREE.Vector3()))
    prev.push(Array.from({ length: segments + 1 }, () => new THREE.Vector3()))
    anchors.push(new THREE.Vector3())
    chainDirs.push(new THREE.Vector3())
  }

  const lateralRest: number[] = []
  for (let i = 0; i <= segments; i++) {
    const v = i / segments
    lateralRest.push((halfWidthAt(options, v) * 2) / (COLUMNS - 1))
  }

  let initialized = false
  let elapsed = 0
  let accumulator = 0
  let smoothedSpeed = 0

  function refreshFrame(): void {
    root.updateWorldMatrix(true, false)
    root.getWorldQuaternion(_rootQuat)
    _rootInv.copy(root.matrixWorld).invert()
    _restWorld.copy(REST_DIR).applyQuaternion(_rootQuat)
    _back.set(0, 0, 1).applyQuaternion(_rootQuat)
    _right.set(1, 0, 0).applyQuaternion(_rootQuat)
    for (let c = 0; c < COLUMNS; c++) {
      anchors[c].copy(chainTops[c]).applyMatrix4(root.matrixWorld)
    }
  }

  function resetToRest(): void {
    refreshFrame()
    for (let c = 0; c < COLUMNS; c++) {
      for (let i = 0; i <= segments; i++) {
        points[c][i].copy(anchors[c]).addScaledVector(_restWorld, segLen * i)
        prev[c][i].copy(points[c][i])
      }
    }
    accumulator = 0
    smoothedSpeed = 0
    initialized = true
  }

  function integrate(dt: number, wind: CapeWind | null): void {
    const dt2 = dt * dt
    const speedBoost = Math.min(
      smoothedSpeed * FLUTTER_SPEED_SCALE,
      FLUTTER_SPEED_CAP
    )
    for (let c = 0; c < COLUMNS; c++) {
      for (let i = 1; i <= segments; i++) {
        const rowT = i / segments
        _accel.set(0, GRAVITY, 0)

        if (wind) {
          const push = wind.windStrength * WIND_ACCEL * (0.5 + 0.5 * rowT)
          _accel.x += wind.windDirX * push
          _accel.z += wind.windDirZ * push
          _accel.y += wind.windStrength * 0.4 * rowT
        }

        const phase = c * 1.7 + i * 0.9
        const flutter = FLUTTER_ACCEL * (0.25 + speedBoost) * rowT
        _accel.addScaledVector(
          _back,
          Math.sin(elapsed * FLUTTER_FREQ + phase) * flutter
        )
        _accel.addScaledVector(
          _right,
          Math.sin(elapsed * FLUTTER_FREQ_LATERAL + phase * 1.3) * flutter * 0.5
        )

        const p = points[c][i]
        _vel.subVectors(p, prev[c][i]).multiplyScalar(DAMPING)
        _next.copy(p).add(_vel).addScaledVector(_accel, dt2)
        prev[c][i].copy(p)
        p.copy(_next)
      }
    }
  }

  function applyConstraints(): void {
    for (let it = 0; it < CONSTRAINT_ITERATIONS; it++) {
      for (let c = 0; c < COLUMNS; c++) {
        for (let i = 1; i <= segments; i++) {
          const a = points[c][i - 1]
          const b = points[c][i]
          _d.subVectors(b, a)
          const len = _d.length()
          if (len < 1e-6) _d.copy(_restWorld)
          else _d.divideScalar(len)

          if (i === 1) clampDirection(_d, _restWorld, MAX_ROOT_BEND)
          else clampDirection(_d, chainDirs[c], MAX_SEGMENT_BEND)

          b.copy(a).addScaledVector(_d, segLen)
          chainDirs[c].copy(_d)
        }
      }

      for (let c = 0; c < COLUMNS - 1; c++) {
        for (let i = 1; i <= segments; i++) {
          const a = points[c][i]
          const b = points[c + 1][i]
          _delta.subVectors(b, a)
          const dist = _delta.length()
          if (dist < 1e-6) continue
          const correction = ((dist - lateralRest[i]) / dist) * 0.5
          a.addScaledVector(_delta, correction)
          b.addScaledVector(_delta, -correction)
        }
      }

      // Stay out of the wearer's back, measured in cape space.
      for (let c = 0; c < COLUMNS; c++) {
        for (let i = 1; i <= segments; i++) {
          const p = points[c][i]
          _local.copy(p).applyMatrix4(_rootInv)
          const minZ = body ? capeMinZAt(body, -_local.y) : 0
          if (_local.z >= minZ) continue
          _local.z = minZ
          p.copy(_local).applyMatrix4(root.matrixWorld)
        }
      }
    }
  }

  function driveBones(): void {
    for (let c = 0; c < COLUMNS; c++) {
      _chainQuat.copy(_rootQuat)
      for (let i = 0; i < segments; i++) {
        _d.subVectors(points[c][i + 1], points[c][i])
        if (_d.lengthSq() < 1e-12) _d.copy(_restWorld)
        else _d.normalize()
        _qInv.copy(_chainQuat).invert()
        _d.applyQuaternion(_qInv)
        const bone = bones[c * segments + i]
        bone.quaternion.setFromUnitVectors(DOWN, _d)
        _chainQuat.multiply(bone.quaternion)
      }
    }
  }

  function update(dt: number, wind: CapeWind | null): void {
    if (!initialized) {
      resetToRest()
      driveBones()
      return
    }
    if (!(dt > 0)) return

    _prevCenter.copy(anchors[1])
    refreshFrame()
    const jump = _prevCenter.distanceTo(anchors[1])
    if (jump > TELEPORT_JUMP) {
      resetToRest()
      driveBones()
      return
    }
    const speed = jump / dt
    smoothedSpeed += (speed - smoothedSpeed) * Math.min(1, dt / SPEED_SMOOTHING)

    elapsed += dt
    accumulator = Math.min(accumulator + dt, MAX_SUBSTEPS * SUBSTEP)
    while (accumulator >= SUBSTEP) {
      accumulator -= SUBSTEP
      for (let c = 0; c < COLUMNS; c++) points[c][0].copy(anchors[c])
      integrate(SUBSTEP, wind)
      applyConstraints()
    }

    driveBones()
  }

  return {
    root,
    mesh,
    update,
    setSkin(skin) {
      if (skinKey(skin) === claimed.key) return
      const next = claimMaterial(skin)
      releaseMaterial(claimed)
      claimed = next
      mesh.material = next.material
    },
    dispose() {
      // Geometry is shared and outlives this rig, and so does the material
      // while anyone still wears the colour; the skeleton is this cape's own.
      releaseMaterial(claimed)
      skeleton.dispose()
      root.removeFromParent()
    },
  }
}

export interface CapeFit {
  /** Bone the cape's root should be added to. */
  parent: THREE.Bone
  position: THREE.Vector3
  quaternion: THREE.Quaternion
  options: CapeRigOptions
}

/** The wearer's own skinned meshes: those sharing the richest skeleton under
 *  `root`. Anything on its own rig — an already-attached cape — is left out, so
 *  it cannot be measured as body surface. */
function findBodyMeshes(root: THREE.Object3D): {
  meshes: THREE.SkinnedMesh[]
  skeleton: THREE.Skeleton | null
} {
  const all: THREE.SkinnedMesh[] = []
  root.traverse((obj) => {
    if (obj instanceof THREE.SkinnedMesh && obj.skeleton) all.push(obj)
  })
  let skeleton: THREE.Skeleton | null = null
  for (const mesh of all) {
    if (!skeleton || mesh.skeleton.bones.length > skeleton.bones.length) {
      skeleton = mesh.skeleton
    }
  }
  return { meshes: all.filter((m) => m.skeleton === skeleton), skeleton }
}

/** Bind-pose world matrix of `skeleton.bones[index]`, from the inverse three
 *  stored when the mesh was bound. Everything the fit measures lives in this
 *  space, so a rig posed by an animation still fits the same way. */
function bindMatrixOfBone(
  skeleton: THREE.Skeleton,
  index: number
): THREE.Matrix4 {
  return skeleton.boneInverses[index].clone().invert()
}

/** Deepest point of the back per height band, in bind space and relative to the
 *  spine bone. Bands are dilated by one so a bulge between samples reads at
 *  both, and empty bands fall back to a body-sized depth. */
function measureBackProfile(
  meshes: THREE.SkinnedMesh[],
  back: THREE.Vector3,
  spinePos: THREE.Vector3,
  topY: number,
  count: number
): Float32Array {
  const raw = new Float32Array(count).fill(-Infinity)

  for (const mesh of meshes) {
    const position = mesh.geometry.getAttribute('position')
    if (!position) continue
    for (let i = 0; i < position.count; i++) {
      _vertex.fromBufferAttribute(position, i).applyMatrix4(mesh.bindMatrix)
      const band = Math.round((topY - _vertex.y) / PROFILE_STEP)
      if (band < 0 || band >= count) continue
      const depth = _vertex.sub(spinePos).dot(back)
      if (depth > raw[band]) raw[band] = depth
    }
  }

  const depths = new Float32Array(count)
  for (let i = 0; i < count; i++) {
    depths[i] = Math.max(
      raw[i],
      raw[Math.max(i - 1, 0)],
      raw[Math.min(i + 1, count - 1)]
    )
    if (!Number.isFinite(depths[i])) depths[i] = FALLBACK_BODY_DEPTH
  }
  return depths
}

/** Build the cape a fit describes and hang it off the bone it names. `color` is
 *  per-wearer, so it rides here rather than in the fit — fits come out of a
 *  cache shared by everyone wearing the same model. */
export function attachCapeFit(fit: CapeFit, skin?: CapeSkin): CapeRig {
  const rig = createCapeRig(
    skin === undefined ? fit.options : { ...fit.options, skin }
  )
  rig.root.position.copy(fit.position)
  rig.root.quaternion.copy(fit.quaternion)
  fit.parent.add(rig.root)
  return rig
}

/** How far the collar sinks into the measured back (`bias`, m) and over what
 *  drop that fades out (`fade`, m). */
export interface CapeCollarTuning {
  bias: number
  fade: number
}

/** Hand-tuned per character model: hair and pauldrons read as body in the
 *  silhouette and hold the cape off the shoulders, so eyeball the bias and
 *  record it here. A short fade sinks only the collar under the hair and gives
 *  the sheet its clearance back just below, so a bending spine has room. */
const COLLAR_TUNING_BY_MODEL: Record<string, Partial<CapeCollarTuning>> = {
  [BARBARIAN_CHARACTER_MODEL_PATH]: { bias: 0.03, fade: 0.1 },
  [CAVEMAN_CHARACTER_MODEL_PATH]: { bias: 0.06 },
  [CAVEWOMAN_CHARACTER_MODEL_PATH]: { bias: 0.06 },
  [FEMALE_BARD_CHARACTER_MODEL_PATH]: { bias: 0.03 },
  [FEMALE_BARBARIAN_CHARACTER_MODEL_PATH]: { bias: 0.1 },
  [FEMALE_KNIGHT_CHARACTER_MODEL_PATH]: { bias: 0.1 },
  [FEMALE_PRIEST_CHARACTER_MODEL_PATH]: { bias: 0.06 },
  [FEMALE_ROGUE_CHARACTER_MODEL_PATH]: { bias: 0.1 },
  [PRIEST_CHARACTER_MODEL_PATH]: { bias: 0.02 },
  [ROGUE_CHARACTER_MODEL_PATH]: { bias: 0.04 },
  [VALKYRIE_CHARACTER_MODEL_PATH]: { bias: 0.22 },
}

const DEFAULT_COLLAR_TUNING: CapeCollarTuning = {
  bias: 0,
  fade: COLLAR_BIAS_FADE,
}

/** Recorded tuning for a character model path; untouched models sit flush. */
export function capeCollarTuningFor(modelPath: string): CapeCollarTuning {
  return { ...DEFAULT_COLLAR_TUNING, ...COLLAR_TUNING_BY_MODEL[modelPath] }
}

/** Measured fits, keyed by model path and tuning. Everything the fit measures
 *  lives in bind space, so every wearer of a model shares it — a crowd costs
 *  one mesh scan, not one per player. Only the spine bone is per-instance. */
const fitCache = new Map<string, Omit<CapeFit, 'parent'>>()

/** Cape frame for a character: hangs off `Spine2`, sized from the arm spread and
 *  torso height, spaced off the back by the mesh's own silhouette. Measured in
 *  bind space and expressed relative to the spine bone, so the cape hugs the
 *  back in every pose. `tuning` sinks the collar into the measured surface —
 *  hair sits in the silhouette and would otherwise float the cape off the
 *  shoulders. Returns null when the rig lacks the bones it needs. */
export function fitCapeToSkeleton(
  characterRoot: THREE.Object3D,
  tuning: CapeCollarTuning = DEFAULT_COLLAR_TUNING,
  modelPath?: string
): CapeFit | null {
  const { meshes, skeleton } = findBodyMeshes(characterRoot)
  if (!skeleton) return null

  const boneIndex = (name: string) =>
    skeleton.bones.findIndex((b) => b.name === name)
  const spineIndex = boneIndex('Spine2')
  const leftIndex = boneIndex('LeftArm')
  const rightIndex = boneIndex('RightArm')
  if (spineIndex < 0 || leftIndex < 0 || rightIndex < 0) return null

  const cacheKey = modelPath && `${modelPath}|${tuning.bias}|${tuning.fade}`
  const cached = cacheKey ? fitCache.get(cacheKey) : undefined
  if (cached) return { ...cached, parent: skeleton.bones[spineIndex] }

  const spineBind = bindMatrixOfBone(skeleton, spineIndex)
  const spinePos = new THREE.Vector3().setFromMatrixPosition(spineBind)
  const leftPos = new THREE.Vector3().setFromMatrixPosition(
    bindMatrixOfBone(skeleton, leftIndex)
  )
  const rightPos = new THREE.Vector3().setFromMatrixPosition(
    bindMatrixOfBone(skeleton, rightIndex)
  )

  const shoulderSpan = leftPos.distanceTo(rightPos)
  // A skeleton bound before its bones had world matrices yields identity
  // inverses; there is no bind pose to fit to.
  if (shoulderSpan < 0.05) return null

  const torsoHeight = Math.max(spinePos.y, 0.5)
  const length = torsoHeight * 0.62

  // right × up points backwards in a right-handed frame (checked against the
  // rigs' toe direction); re-deriving up keeps the basis orthonormal when the
  // arms sit at different heights.
  const right = rightPos.clone().sub(leftPos).normalize()
  const back = right.clone().cross(UP).normalize()
  const up = back.clone().cross(right).normalize()

  const collarLift = torsoHeight * 0.07
  const collarY = spinePos.y + collarLift
  const count = Math.max(2, Math.ceil(length / PROFILE_STEP) + 1)
  const depths = measureBackProfile(meshes, back, spinePos, collarY, count)

  // Hang the collar just clear of the back at collar height — the per-height
  // profile below pushes the sheet out over the shoulder blades and buttocks,
  // so the collar itself does not have to stand off for them. The bias then
  // sinks the top of the sheet into the surface, fading out down the length.
  const bias = THREE.MathUtils.clamp(
    tuning.bias,
    -COLLAR_BIAS_LIMIT,
    COLLAR_BIAS_LIMIT
  )
  const collarDepth = depths[0] + SURFACE_CLEARANCE - bias
  const minZ = new Float32Array(count)
  for (let i = 0; i < count; i++) {
    const fade = bias * Math.max(0, 1 - (i * PROFILE_STEP) / tuning.fade)
    minZ[i] = depths[i] - collarDepth - fade
  }

  const anchorBind = spinePos
    .clone()
    .addScaledVector(up, collarLift)
    .addScaledVector(back, collarDepth)

  const basis = new THREE.Matrix4().makeBasis(right, up, back)
  const spineBindQuat = new THREE.Quaternion().setFromRotationMatrix(spineBind)

  const measured = {
    position: anchorBind.applyMatrix4(spineBind.clone().invert()),
    quaternion: new THREE.Quaternion()
      .setFromRotationMatrix(basis)
      .premultiply(spineBindQuat.invert()),
    options: {
      topWidth: shoulderSpan * 0.95,
      bottomWidth: shoulderSpan * 1.35,
      length,
      body: { minZ, step: PROFILE_STEP },
    },
  }
  if (cacheKey) fitCache.set(cacheKey, measured)
  return { ...measured, parent: skeleton.bones[spineIndex] }
}

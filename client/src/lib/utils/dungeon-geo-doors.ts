/**
 * dungeon-geo-doors.ts — door and arch geometry for the dungeon: the shared cap
 * arc / dome / arch shape helpers, the swinging half-heptagon leaves, and the
 * surface-entrance and interior-room door assemblies. The arch quads merge
 * statically into the caller's group; the leaves are returned so the layer can
 * animate them (collision comes from the wasm passability cells).
 */
import * as THREE from 'three'
import { mergeVertices } from 'three/examples/jsm/utils/BufferGeometryUtils.js'
import type { GeoEntry } from './house-geo-utils'
import { getGhostHousingMaterial, getHousingMaterial } from './housing-textures'
import {
  ENTRANCE_WALL_T,
  ENTRANCE_DOOR_DEPTH,
  ENTRANCE_DOOR_ID,
  type DungeonShaft,
  type InteriorDoorSpec,
} from '../managers/dungeonManager'
import {
  WALL_THICKNESS,
  WALL_HALF_THICKNESS,
  DUNGEON_DOOR_TEXTURE_IDX,
  DUNGEON_LOCKED_DOOR_TEXTURE_IDX,
  DUNGEON_ENTRANCE_WALL_TEXTURE_IDX,
  DUNGEON_WALL_TEXTURE_IDX,
  WALL_N,
  WALL_E,
  WALL_S,
  WALL_W,
  type DungeonWall,
  type DungeonGeoCtx,
} from './dungeon-geo-constants'

/** Door panel thickness (m). */
const DOOR_THICKNESS = 0.12
/** Door apex height above the entry ground (under the ABOVE=3.0 doorway). */
const DOOR_HEIGHT = 2.85
/** Height where the rectangular body ends and the pentagonal cap begins. */
const DOOR_SHOULDER = 1.9

/**
 * Half-door leaf outline: the hinge-side half of the door, in the leaf's local
 * XY plane with the hinge edge at x=0, the centre split at x=halfW, and the
 * bottom at y=0. The outer (hinge) edge runs straight up to the shoulder, then
 * a two-segment arc (bend1 lower, bend2 upper) on a quarter-ellipse curves over
 * to the apex on the centre split — giving a rounded dome rather than a steep
 * pointed gable. Two mirrored leaves form the full door:
 *
 *          _apex_      _apex_
 *        bend2  |      |  bend2   ← rounded cap (split down the middle)
 *       bend1   |      |   bend1
 *   shoulder    |      |    shoulder ← body top
 *       |       |      |       |    ← rectangular body
 *    hinge --- split  split --- hinge
 */
/**
 * Intermediate angles (deg) of the cap arc between the shoulder (0°) and the
 * apex (90°). Shared by the door leaves and the entrance arch so their curves
 * coincide exactly.
 */
const DOOR_CAP_ANGLES_DEG = [30, 60]

/**
 * A point on the cap's quarter-ellipse measured from the outer edge: x =
 * halfW·(1−cosθ) runs 0→halfW, y = shoulder + capH·sinθ runs shoulder→shoulder+capH.
 */
function capArcPoint(
  halfW: number,
  capH: number,
  shoulder: number,
  deg: number
): { x: number; y: number } {
  const t = (deg * Math.PI) / 180
  return { x: halfW * (1 - Math.cos(t)), y: shoulder + capH * Math.sin(t) }
}

function halfDoorLeafShape(
  halfW: number,
  h: number,
  shoulder: number
): THREE.Shape {
  const s = new THREE.Shape()
  const capH = h - shoulder
  const bend1 = capArcPoint(halfW, capH, shoulder, DOOR_CAP_ANGLES_DEG[0])
  const bend2 = capArcPoint(halfW, capH, shoulder, DOOR_CAP_ANGLES_DEG[1])
  s.moveTo(0, 0) // bottom hinge corner
  s.lineTo(halfW, 0) // bottom centre (split)
  s.lineTo(halfW, h) // apex (top of split)
  s.lineTo(bend2.x, bend2.y) // upper cap bend
  s.lineTo(bend1.x, bend1.y) // lower cap bend
  s.lineTo(0, shoulder) // hinge shoulder
  s.closePath()
  return s
}

/**
 * Front entrance wall filling the entry opening above the door's rounded cap.
 * Built as an extruded shape, bottom→top: the dome curve (matching the door cap
 * exactly) as its arched underside, vertical sides up to the wall top, then a
 * gabled peak rising `gableRise` to the roof ridge — so this single wall fills
 * both the arch spandrel and the front gable triangle (which the roof therefore
 * omits), all in the wall texture. Rotated/translated onto the entry plane and
 * pushed as a wall-textured GeoEntry to merge with the other entrance walls.
 * `entryLow` = the entry sits at the low-coordinate end.
 */
/** Append the door-cap dome underside (right→left) to an arch shape spanning
 *  [0,W] laterally, so the rounded door cap fits exactly beneath it. */
function traceArchDome(shape: THREE.Shape, W: number) {
  const halfW = W / 2
  const capH = DOOR_HEIGHT - DOOR_SHOULDER
  const p1 = capArcPoint(halfW, capH, DOOR_SHOULDER, DOOR_CAP_ANGLES_DEG[0])
  const p2 = capArcPoint(halfW, capH, DOOR_SHOULDER, DOOR_CAP_ANGLES_DEG[1])
  shape.lineTo(W - p1.x, p1.y) // right lower bend
  shape.lineTo(W - p2.x, p2.y) // right upper bend
  shape.lineTo(halfW, DOOR_HEIGHT) // apex
  shape.lineTo(p2.x, p2.y) // left upper bend
  shape.lineTo(p1.x, p1.y) // left lower bend
}

/**
 * Extrude an arch spandrel shape (laid out in lateral-X / up-Y) to thickness
 * `T`, convert to indexed (so it merges with the indexed wall geometry), place
 * it on the wall line and push it. `alongZ` ⇒ keep shapeX→X with thickness→Z
 * centred at z=`wallLine`; otherwise rotate shapeX→+Z with thickness→−X centred
 * at x=`wallLine`. `latStart` is the lateral origin (the opening's low edge).
 */
function extrudeAndPlaceArch(
  entries: GeoEntry[],
  shape: THREE.Shape,
  alongZ: boolean,
  latStart: number,
  wallLine: number,
  T: number,
  textureIndex: number
) {
  const raw = new THREE.ExtrudeGeometry(shape, {
    depth: T,
    bevelEnabled: false,
  })
  const geo = mergeVertices(raw)
  raw.dispose()
  const m = new THREE.Matrix4()
  if (alongZ) {
    m.makeTranslation(latStart, 0, wallLine - T / 2)
    geo.applyMatrix4(m)
  } else {
    m.makeRotationY(-Math.PI / 2)
    geo.applyMatrix4(m)
    m.makeTranslation(wallLine + T / 2, 0, latStart)
    geo.applyMatrix4(m)
  }
  entries.push({ geo, textureIndex })
}

export function addEntranceArch(
  entries: GeoEntry[],
  alongZ: boolean,
  cr: { x: number; w: number; z: number; d: number },
  entryLow: boolean,
  ctx: DungeonGeoCtx,
  top: number,
  gableRise: number
) {
  const W = ctx.shaftW
  // Gabled top + sides, then the shared dome underside.
  const shape = new THREE.Shape()
  shape.moveTo(0, DOOR_SHOULDER)
  shape.lineTo(0, top)
  shape.lineTo(W / 2, top + gableRise) // gable apex (roof ridge)
  shape.lineTo(W, top)
  shape.lineTo(W, DOOR_SHOULDER)
  traceArchDome(shape, W)
  shape.closePath()
  // alongZ: entry plane is z; otherwise x. entryLow ⇒ the low-coordinate end.
  extrudeAndPlaceArch(
    entries,
    shape,
    alongZ,
    alongZ ? cr.x : cr.z,
    alongZ ? (entryLow ? cr.z : cr.z + cr.d) : entryLow ? cr.x : cr.x + cr.w,
    ENTRANCE_WALL_T,
    DUNGEON_ENTRANCE_WALL_TEXTURE_IDX
  )
}

export interface DoorLeaf {
  pivot: THREE.Group
  mesh: THREE.Mesh
  /** rotation.y when shut (leaf flush across its half of the doorway). */
  closedAngle: number
  /** rotation.y when fully open (swung outward); within ±90° of closed. */
  openAngle: number
}

/**
 * Open angle for a leaf: closedAngle ± 90°, choosing the sign whose swing
 * points the leaf outward (a ≤90° swing, so a linear lerp never wraps the long
 * way round). A leaf's +x' axis points to (cosφ, 0, −sinφ) after rotation.y=φ.
 */
function leafOpenAngle(
  closedAngle: number,
  outX: number,
  outZ: number
): number {
  // The two candidates are 180° apart, so their outward dot products are exact
  // negatives — a single sign test on the +90° candidate picks the outward one.
  const plus = closedAngle + Math.PI / 2
  const dotPlus = Math.cos(plus) * outX - Math.sin(plus) * outZ
  return dotPlus >= 0 ? plus : closedAngle - Math.PI / 2
}

/**
 * Build one door leaf: a half-heptagon panel extruded to DOOR_THICKNESS and
 * hinged at its outer jamb, mapped to its half of the garage-door image (u from
 * the hinge edge to the centre split, v over the full height). Shared by the
 * surface entrance and the interior room doors. `outX/outZ` is the outward swing
 * direction (the side the leaves open toward). The pivot is left pickable for
 * the click raycaster; callers tag `pivot.userData.dungeonDoorKey` with the
 * door's (depth, id) so a click resolves which door to toggle.
 */
interface DoorLeafSpec {
  hingeX: number
  hingeZ: number
  closedAngle: number
  uHinge: number
}

/**
 * The two leaf hinge specs for a double door. `alongZ` ⇒ the door spans X at the
 * wall line z=`line` (low/high jambs at x=`latLow`/`latHigh`); otherwise it spans
 * Z at x=`line`. The leaves hinge on opposite jambs and meet at the centre split
 * (uHinge 0/1 → both map to U=0.5 there). Shared by the entrance (its along-Z
 * case ≙ a room's north wall) and the interior doors.
 */
function doorLeafSpecs(
  alongZ: boolean,
  latLow: number,
  latHigh: number,
  line: number
): DoorLeafSpec[] {
  return alongZ
    ? [
        { hingeX: latLow, hingeZ: line, closedAngle: 0, uHinge: 0 },
        { hingeX: latHigh, hingeZ: line, closedAngle: Math.PI, uHinge: 1 },
      ]
    : [
        { hingeX: line, hingeZ: latLow, closedAngle: -Math.PI / 2, uHinge: 0 },
        { hingeX: line, hingeZ: latHigh, closedAngle: Math.PI / 2, uHinge: 1 },
      ]
}

function makeDoorLeaf(
  spec: DoorLeafSpec,
  halfW: number,
  outX: number,
  outZ: number,
  mat: THREE.Material
): DoorLeaf {
  const shape = halfDoorLeafShape(halfW, DOOR_HEIGHT, DOOR_SHOULDER)
  const geo = new THREE.ExtrudeGeometry(shape, {
    depth: DOOR_THICKNESS,
    bevelEnabled: false,
  })
  geo.translate(0, 0, -DOOR_THICKNESS / 2) // centre thickness on the hinge plane
  // ExtrudeGeometry UVs are in shape (meter) coords. Map this leaf to its half
  // of the image: u runs from uHinge (hinge edge) to 0.5 (centre split), v spans
  // the full height. (Thin side faces get squished UVs — barely seen.)
  const uv = geo.getAttribute('uv')
  for (let i = 0; i < uv.count; i++) {
    const u = spec.uHinge + (0.5 - spec.uHinge) * (uv.getX(i) / halfW)
    uv.setXY(i, u, uv.getY(i) / DOOR_HEIGHT)
  }
  uv.needsUpdate = true

  const pivot = new THREE.Group()
  pivot.position.set(spec.hingeX, 0, spec.hingeZ)
  pivot.rotation.y = spec.closedAngle
  const mesh = new THREE.Mesh(geo, mat)
  pivot.add(mesh)
  return {
    pivot,
    mesh,
    closedAngle: spec.closedAngle,
    openAngle: leafOpenAngle(spec.closedAngle, outX, outZ),
  }
}

/** Tag a door's leaves so the click raycaster knows which door to toggle. */
function tagDoorLeaves(leaves: DoorLeaf[], depth: number, doorId: number) {
  for (const leaf of leaves) {
    leaf.pivot.userData.dungeonDoorKey = { depth, doorId }
  }
}

/**
 * Double entrance doors, split down the middle and swinging open to both sides
 * like a house door. Two pivot Groups (each rotating about its outer hinge),
 * each carrying a half-heptagon leaf mesh; positioned at the open (entry) end
 * of the covered footprint, at ground level (local y=0). The two leaves' UVs
 * map the left/right halves of the garage-door image so they reconstruct one
 * door when shut. Caller animates each `pivot.rotation.y` between the returned
 * closed/open angles (open swings outward, away from the deep end).
 */
export function buildEntranceDoors(
  entranceShaft: DungeonShaft,
  cr: { x: number; w: number; z: number; d: number },
  ctx: DungeonGeoCtx
): DoorLeaf[] {
  const halfW = ctx.shaftW / 2
  const alongZ = entranceShaft.alongZ
  const nonrev = !entranceShaft.reversed
  // Outward (toward the entry/outside, away from the deep end).
  const outX = alongZ ? 0 : nonrev ? -1 : 1
  const outZ = alongZ ? (nonrev ? -1 : 1) : 0
  // Entry (open) end is the low-coordinate end unless the shaft runs reversed.
  const entryZ = nonrev ? cr.z : cr.z + cr.d
  const entryX = nonrev ? cr.x : cr.x + cr.w

  const mat = getHousingMaterial(DUNGEON_DOOR_TEXTURE_IDX)

  // The two leaves hinge on opposite lateral jambs (low / high) and meet at the
  // doorway centre. `uHinge` is the image U at the hinge edge; both leaves run
  // to U=0.5 at the split, so the low leaf maps [0,0.5] and the high leaf [1,0.5].
  const specs = doorLeafSpecs(
    alongZ,
    alongZ ? cr.x : cr.z,
    alongZ ? cr.x + cr.w : cr.z + cr.d,
    alongZ ? entryZ : entryX
  )

  const leaves = specs.map((spec) => makeDoorLeaf(spec, halfW, outX, outZ, mat))
  for (const leaf of leaves) leaf.pivot.name = 'dungeon_entrance_door'
  tagDoorLeaves(leaves, ENTRANCE_DOOR_DEPTH, ENTRANCE_DOOR_ID)
  return leaves
}

export interface InteriorDoor {
  /** Synced-state key (matches the toggle packet + dungeonManager door map). */
  depth: number
  doorId: number
  /** The two swinging leaves (added to a sibling group, animated by the layer). */
  leaves: DoorLeaf[]
  /** Eased open fraction (0 shut .. 1 open); the layer advances it toward the
   *  click-toggled, server-synced open state. */
  open: number
  /** Leaf materials for the occlusion fade, like a wall run's. */
  base: THREE.Material
  ghost: THREE.Material
  /** World AABB of the leaves; the layer refreshes it while they swing. */
  aabb: THREE.Box3
}

/**
 * Stone arch above an interior room door: a flat-topped wall panel spanning the
 * opening from the door's shoulder up to the wall top, its underside the same
 * quarter-ellipse dome as the door cap (so the rounded door fits beneath it).
 * Room-wall texture (matching the surrounding room walls), extruded to the
 * wall-run thickness and centred on the room↔corridor wall line. `spansX`
 * ⇒ the door spans X (panel laid along X, thickness along Z); otherwise it spans
 * Z (rotated so the panel lies along Z, thickness along X).
 */
function addInteriorDoorArch(
  archEntries: GeoEntry[],
  spansX: boolean,
  lat0: number,
  W: number,
  wallLine: number,
  wallTop: number
) {
  // Flat top + sides, then the shared dome underside.
  const shape = new THREE.Shape()
  shape.moveTo(0, DOOR_SHOULDER)
  shape.lineTo(0, wallTop)
  shape.lineTo(W, wallTop)
  shape.lineTo(W, DOOR_SHOULDER)
  traceArchDome(shape, W)
  shape.closePath()
  extrudeAndPlaceArch(
    archEntries,
    shape,
    spansX,
    lat0,
    wallLine,
    WALL_THICKNESS,
    DUNGEON_WALL_TEXTURE_IDX
  )
}

interface DoorWallInfo {
  /** Door runs along X (north/south walls) rather than Z (east/west walls). */
  spansX: boolean
  /** Wall sits at the room's low-coordinate edge (north at min z, west at min
   *  x), so its run is half a wall-thickness below the grid line, not above. */
  outerLow: boolean
  /** Unit direction the leaves swing as they open into the room. */
  outX: number
  outZ: number
}

/** Per-wall swing direction and wall-plane side for `buildInteriorDoor`. */
const DOOR_WALL_INFO: Record<DungeonWall, DoorWallInfo> = {
  [WALL_N]: { spansX: true, outerLow: true, outX: 0, outZ: 1 },
  [WALL_E]: { spansX: false, outerLow: false, outX: -1, outZ: 0 },
  [WALL_S]: { spansX: true, outerLow: false, outX: 0, outZ: -1 },
  [WALL_W]: { spansX: false, outerLow: true, outX: 1, outZ: 0 },
}

/**
 * One interior room door across a corridor mouth (placement from a wasm
 * `InteriorDoorSpec`): a pair of swinging leaves (same half-heptagon shape as
 * the surface entrance doors) plus a stone arch filling the wall above the
 * rounded cap. Arch quads go to `archEntries` (room-wall texture, merged
 * statically); the returned leaves are added to the floor group and animated.
 */
export function buildInteriorDoor(
  depth: number,
  spec: InteriorDoorSpec,
  wallTop: number,
  archEntries: GeoEntry[]
): InteriorDoor {
  const textureIndex = spec.locked
    ? DUNGEON_LOCKED_DOOR_TEXTURE_IDX
    : DUNGEON_DOOR_TEXTURE_IDX
  const mat = getHousingMaterial(textureIndex)
  const { lat0, len, wallLine, doorId } = spec
  const halfW = len / 2
  const { spansX, outerLow, outX, outZ } = DOOR_WALL_INFO[spec.wall]
  // The room wall run sits half its thickness outside the room boundary: below
  // the grid line for low-coordinate walls (north/west), above for high
  // (south/east). Place the visible leaves and arch on that plane to keep them
  // flush with the wall; the door id and collision stay on the grid line.
  const wallPlane =
    wallLine + (outerLow ? -WALL_HALF_THICKNESS : WALL_HALF_THICKNESS)
  // spansX ≙ the entrance's along-Z case (door spans X at z=wallPlane).
  const leafSpecs = doorLeafSpecs(spansX, lat0, lat0 + len, wallPlane)
  const leaves = leafSpecs.map((s) => makeDoorLeaf(s, halfW, outX, outZ, mat))
  tagDoorLeaves(leaves, depth, doorId)
  addInteriorDoorArch(archEntries, spansX, lat0, len, wallPlane, wallTop)
  return {
    depth,
    doorId,
    leaves,
    open: 0,
    base: mat,
    ghost: getGhostHousingMaterial(textureIndex),
    aabb: new THREE.Box3(),
  }
}

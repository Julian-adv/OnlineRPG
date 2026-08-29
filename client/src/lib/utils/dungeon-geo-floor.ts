/**
 * dungeon-geo-floor.ts — the renderable group for one underground dungeon floor:
 * the floor slab (minus the down-shaft hole), the up/down stair shafts, the
 * per-run fade walls, and the interior room doors. Follows the
 * housing pattern: collect GeoEntry quads per texture, merge into one mesh per
 * texture, reuse the shared housing materials.
 */
import * as THREE from 'three'
import { addMergedMeshes, type GeoEntry } from './house-geo-utils'
import { getHousingMaterial } from './housing-textures'
import type {
  DungeonFloorLayout,
  InteriorDoorSpec,
} from '../managers/dungeonManager'
import { addBox, quadMeshBuilder } from './dungeon-geo-primitives'
import {
  shaftRect,
  rectContains,
  shaftContains,
  shaftStepCell,
  collectShaftStairs,
} from './dungeon-geo-shaft'
import { buildInteriorDoor, type InteriorDoor } from './dungeon-geo-doors'
import {
  DUNGEON_FLOOR_TEXTURE_IDX,
  DUNGEON_WALL_TEXTURE_IDX,
  DUNGEON_CORRIDOR_WALL_TEXTURE_IDX,
  DUNGEON_DOOR_TEXTURE_IDX,
  SLAB_THICKNESS,
  DUNGEON_FLOOR_UV_SCALE,
  SHADOW_CONTACT_LIFT,
  WALL_THICKNESS,
  WALL_HALF_THICKNESS,
  UP_SHAFT_GROUP_NAME,
  type DungeonGeoCtx,
} from './dungeon-geo-constants'

/** Scene-graph name of the wall-run sub-group (all four sides), for debugging.
 *  Unlike the up-shaft group it is never looked up by name — the layer caches
 *  the runs from the returned WallRun[] — so it stays module-private. */
const WALL_RUN_GROUP_NAME = 'wallRuns'

/** One straight wall run, built as its own mesh so the dungeon layer can fade
 *  just this run to a ghost when it occludes the player. */
export interface WallRun {
  mesh: THREE.Mesh
  /** Group-local AABB; the layer adds the floor group's world position. */
  localAABB: THREE.Box3
}

export interface DungeonFloorGroup {
  group: THREE.Group
  /** Local-space AABB of the up-shaft stairs sub-group, for the layer's
   *  occlusion-fade test (add the group's world position to use it). */
  upShaftAABB: THREE.Box3
  /** Per-side wall runs (all four directions), faded individually on occlusion. */
  wallRuns: WallRun[]
  /** Interior room doors at corridor mouths, animated by the layer. */
  doors: InteriorDoor[]
}

/** Plain door material tinted toward iron so a keyed door reads as locked. */
let lockedDoorMaterial: THREE.MeshStandardMaterial | null = null
function getLockedDoorMaterial(
  base: THREE.MeshStandardMaterial
): THREE.Material {
  if (!lockedDoorMaterial) {
    lockedDoorMaterial = base.clone()
    lockedDoorMaterial.color.setRGB(0.45, 0.42, 0.42)
    lockedDoorMaterial.metalness = 0.5
  }
  return lockedDoorMaterial
}

/**
 * Build the renderable group for one dungeon floor. The caller positions
 * it at (originX, floorY(depth), originZ) in world space. `doorSpecs` is the
 * floor's interior-door placement list from `dungeonManager.interiorDoorsAt`.
 */
export function buildDungeonFloorGroup(
  layout: DungeonFloorLayout,
  ctx: DungeonGeoCtx,
  doorSpecs: InteriorDoorSpec[]
): DungeonFloorGroup {
  const grid = ctx.grid
  const carvedAt = (x: number, z: number) =>
    x >= 0 && x < grid && z >= 0 && z < grid && layout.carved[x + z * grid]

  const entries: GeoEntry[] = []

  // Down-shaft hole: slab is omitted over the shaft except its entry row.
  const down = layout.downShaft
  const downEntry = down ? shaftStepCell(down, ctx, 0, 0) : null
  const inDownHole = (x: number, z: number): boolean => {
    if (!down || !shaftContains(down, ctx, x, z)) return false
    const onEntryRow = down.alongZ ? z === downEntry!.z : x === downEntry!.x
    return !onEntryRow
  }
  // Note: serde Option<T> arrives as undefined (not null) over wasm.
  const inAnyShaft = (x: number, z: number): boolean =>
    shaftContains(layout.upShaft, ctx, x, z) ||
    (down != null && shaftContains(down, ctx, x, z))

  // --- Floor slab: seamless top/bottom run quads with skirts only at true
  // edges. Full boxes' interior seam faces reach y=0 with zero depth margin
  // under BackSide shadow rendering and flicker as acne past ~5m of the torch;
  // seamless, SLAB_THICKNESS is the self-bias and the slab can keep casting.
  // A run also breaks where a side's openness flips, so every skirt shares its
  // corners with the top face (no T-junction cracks).
  const solidAt = (x: number, z: number) => carvedAt(x, z) && !inDownHole(x, z)
  const slab = quadMeshBuilder(DUNGEON_FLOOR_UV_SCALE)
  const v = (x: number, y: number, z: number) => new THREE.Vector3(x, y, z)
  const yB = -SLAB_THICKNESS
  for (let z = 0; z < grid; z++) {
    let runStart = -1
    let runN = false
    let runS = false
    for (let x = 0; x <= grid; x++) {
      const solid = x < grid && solidAt(x, z)
      const nOpen = solid && !solidAt(x, z - 1)
      const sOpen = solid && !solidAt(x, z + 1)
      if (runStart >= 0 && (!solid || nOpen !== runN || sOpen !== runS)) {
        const x0 = runStart
        const cap = (y: number, n: THREE.Vector3) =>
          slab.addQuad(
            v(x0, y, z),
            v(x, y, z),
            v(x, y, z + 1),
            v(x0, y, z + 1),
            n
          )
        const skirtZ = (za: number, n: THREE.Vector3) =>
          slab.addQuad(
            v(x0, yB, za),
            v(x, yB, za),
            v(x, 0, za),
            v(x0, 0, za),
            n
          )
        const skirtX = (xa: number, n: THREE.Vector3) =>
          slab.addQuad(
            v(xa, yB, z),
            v(xa, yB, z + 1),
            v(xa, 0, z + 1),
            v(xa, 0, z),
            n
          )
        cap(0, v(0, 1, 0))
        cap(yB, v(0, -1, 0))
        if (runN) skirtZ(z, v(0, 0, -1))
        if (runS) skirtZ(z + 1, v(0, 0, 1))
        if (!solidAt(x0 - 1, z)) skirtX(x0, v(-1, 0, 0))
        if (!solidAt(x, z)) skirtX(x, v(1, 0, 0))
        runStart = -1
      }
      if (solid && runStart < 0) {
        runStart = x
        runN = nOpen
        runS = sOpen
      }
    }
  }
  slab.finish(entries, DUNGEON_FLOOR_TEXTURE_IDX)

  // Walls on all four sides are built lower down as per-run fade meshes (the
  // dungeon layer ghosts any run that occludes the player), not merged here.

  // --- Down shaft (0 → -floorHeight) merges with the floor geometry, so it
  // shares the slab's y=0 and isn't lifted for shadow contact like the up-shaft;
  // any peter-panning at the hole's top edge is deferred (split it out to fix).
  if (down) {
    collectShaftStairs(entries, down, ctx, 0, -ctx.floorHeight, false, true)
  }

  const group = new THREE.Group()
  addMergedMeshes(group, entries)

  // --- Up shaft (descends from the floor above, +floorHeight → 0): the
  // staircase you arrive by. Built into its own sub-group so the dungeon layer
  // can fade it to a ghost material when it occludes the player from the iso
  // camera (it shares the floor texture, so it can't fade while merged in).
  // Its side wall is omitted: the steps are blocked by an impassable flag, so
  // no wall is needed to contain the player, and a wall would only block the
  // view down the stairs.
  const upEntries: GeoEntry[] = []
  collectShaftStairs(
    upEntries,
    layout.upShaft,
    ctx,
    ctx.floorHeight,
    0,
    true, // top landing: neighbour floor's slab is not rendered
    false, // bottom landing: this floor's slab covers the exit row
    false // no side wall
  )
  const upGroup = new THREE.Group()
  upGroup.name = UP_SHAFT_GROUP_NAME
  // Lift off the floor (SHADOW_CONTACT_LIFT); collision uses the server ramp Y, so
  // the sub-centimetre visual shift doesn't affect it.
  upGroup.position.y = SHADOW_CONTACT_LIFT
  addMergedMeshes(upGroup, upEntries)
  group.add(upGroup)

  // --- Wall runs (all four sides): one mesh per straight run (a run breaks at
  // a corner, a doorway gap or a shaft cell) in their own sub-group, so the
  // dungeon layer can ghost just the runs that occlude the player. The
  // camera-facing south/west runs fade often; the far north/east runs fade only
  // when the layout puts them between the iso camera and the player. They are
  // decorative — collision is server-side — so they cast no shadow (a ghosted
  // run would otherwise drop a wall-less shadow on the floor) and ignore click
  // raycasts (so click-to-move targets the floor behind them).
  const wallRunGroup = new THREE.Group()
  wallRunGroup.name = WALL_RUN_GROUP_NAME
  const wallRuns: WallRun[] = []
  const addWallRun = (
    texIdx: number,
    w: number,
    h: number,
    d: number,
    cx: number,
    cy: number,
    cz: number
  ) => {
    const e: GeoEntry[] = []
    addBox(e, texIdx, w, h, d, cx, cy, cz)
    const geo = e[0].geo
    const mesh = new THREE.Mesh(geo, getHousingMaterial(texIdx))
    mesh.castShadow = false
    mesh.receiveShadow = true
    mesh.raycast = () => {}
    mesh.userData.textureIndex = texIdx
    geo.computeBoundingBox()
    wallRunGroup.add(mesh)
    wallRuns.push({ mesh, localAABB: geo.boundingBox!.clone() })
  }
  // Room cells keep the medieval-stone wall; carved cells in no room are
  // corridors and get the rock-wall corridor texture. Matches the Rust
  // `cell_in_any_room` convention (half-open rectangles). Shaft cells emit no
  // wall (filtered by `inAnyShaft`), so they need no classification here.
  const roomAt = (x: number, z: number) =>
    layout.rooms.some((r) => rectContains(r, x, z))
  const wallTexAt = (x: number, z: number) =>
    roomAt(x, z) ? DUNGEON_WALL_TEXTURE_IDX : DUNGEON_CORRIDOR_WALL_TEXTURE_IDX
  // Pull a corridor run in by the wall thickness at each end where a perpendicular
  // room wall crosses, so the two coplanar faces don't z-fight (the room wall keeps
  // the corner). `diagLo`/`diagHi` are the cells just past each end on the wall
  // side; a room cell there means a room wall crosses. Colinear continuations and
  // corridor↔corridor corners have a solid/corridor cell there, so they stay
  // full-length (no gap). Returns the trimmed run span [lo, hi].
  const trimCorridorRun = (
    tex: number,
    lo: number,
    hi: number,
    diagLo: [number, number],
    diagHi: [number, number]
  ): [number, number] =>
    tex !== DUNGEON_CORRIDOR_WALL_TEXTURE_IDX
      ? [lo, hi]
      : [
          roomAt(diagLo[0], diagLo[1]) ? lo + WALL_THICKNESS : lo,
          roomAt(diagHi[0], diagHi[1]) ? hi - WALL_THICKNESS : hi,
        ]
  // North/south edges merge into x-runs (one wall per row); the wall sits just
  // past the carved cell's north (z − HALF) or south (z + 1 + HALF) face.
  for (let z = 0; z < grid; z++) {
    let northStart = -1
    let northTex = -1
    let southStart = -1
    let southTex = -1
    for (let x = 0; x <= grid; x++) {
      const carved = x < grid && carvedAt(x, z) && !inAnyShaft(x, z)
      const north = carved && !carvedAt(x, z - 1)
      const south = carved && !carvedAt(x, z + 1)
      const tex = carved ? wallTexAt(x, z) : -1
      // Close a run at a gap, corner, or where room↔corridor texture flips.
      if (northStart >= 0 && (!north || tex !== northTex)) {
        const [lo, hi] = trimCorridorRun(
          northTex,
          northStart,
          x,
          [northStart - 1, z - 1],
          [x, z - 1]
        )
        const len = hi - lo
        addWallRun(
          northTex,
          len,
          ctx.wallHeight,
          WALL_THICKNESS,
          lo + len / 2,
          ctx.wallHeight / 2 + SHADOW_CONTACT_LIFT,
          z - WALL_HALF_THICKNESS
        )
        northStart = -1
      }
      if (north && northStart < 0) {
        northStart = x
        northTex = tex
      }
      if (southStart >= 0 && (!south || tex !== southTex)) {
        const [lo, hi] = trimCorridorRun(
          southTex,
          southStart,
          x,
          [southStart - 1, z + 1],
          [x, z + 1]
        )
        const len = hi - lo
        addWallRun(
          southTex,
          len,
          ctx.wallHeight,
          WALL_THICKNESS,
          lo + len / 2,
          ctx.wallHeight / 2 + SHADOW_CONTACT_LIFT,
          z + 1 + WALL_HALF_THICKNESS
        )
        southStart = -1
      }
      if (south && southStart < 0) {
        southStart = x
        southTex = tex
      }
    }
  }
  // East/west edges merge into z-runs; the wall sits just past the carved cell's
  // east (x + 1 + HALF) or west (x − HALF) face.
  for (let x = 0; x < grid; x++) {
    let eastStart = -1
    let eastTex = -1
    let westStart = -1
    let westTex = -1
    for (let z = 0; z <= grid; z++) {
      const carved = z < grid && carvedAt(x, z) && !inAnyShaft(x, z)
      const east = carved && !carvedAt(x + 1, z)
      const west = carved && !carvedAt(x - 1, z)
      const tex = carved ? wallTexAt(x, z) : -1
      if (eastStart >= 0 && (!east || tex !== eastTex)) {
        const [lo, hi] = trimCorridorRun(
          eastTex,
          eastStart,
          z,
          [x + 1, eastStart - 1],
          [x + 1, z]
        )
        const len = hi - lo
        addWallRun(
          eastTex,
          WALL_THICKNESS,
          ctx.wallHeight,
          len,
          x + 1 + WALL_HALF_THICKNESS,
          ctx.wallHeight / 2 + SHADOW_CONTACT_LIFT,
          lo + len / 2
        )
        eastStart = -1
      }
      if (east && eastStart < 0) {
        eastStart = z
        eastTex = tex
      }
      if (westStart >= 0 && (!west || tex !== westTex)) {
        const [lo, hi] = trimCorridorRun(
          westTex,
          westStart,
          z,
          [x - 1, westStart - 1],
          [x - 1, z]
        )
        const len = hi - lo
        addWallRun(
          westTex,
          WALL_THICKNESS,
          ctx.wallHeight,
          len,
          x - WALL_HALF_THICKNESS,
          ctx.wallHeight / 2 + SHADOW_CONTACT_LIFT,
          lo + len / 2
        )
        westStart = -1
      }
      if (west && westStart < 0) {
        westStart = z
        westTex = tex
      }
    }
  }
  group.add(wallRunGroup)

  // --- Interior room doors, placed by the shared wasm scan (see the Rust
  // `dungeon::doors` module doc). Arches merge statically; the swinging
  // leaves are returned for the layer to animate.
  const doorMat = getHousingMaterial(DUNGEON_DOOR_TEXTURE_IDX)
  const archEntries: GeoEntry[] = []
  const doors: InteriorDoor[] = doorSpecs.map((spec) =>
    buildInteriorDoor(
      layout.depth,
      spec,
      ctx.wallHeight,
      spec.locked ? getLockedDoorMaterial(doorMat) : doorMat,
      archEntries
    )
  )
  // Arches merge into the floor group but stay non-pickable, so a ground click
  // near a doorway falls through to the floor. The door leaves are NOT added
  // here — the layer parents them to a separate pickable group (so the door
  // click raycast can hit them without them intercepting click-to-move).
  if (archEntries.length > 0) {
    const archGroup = new THREE.Group()
    addMergedMeshes(archGroup, archEntries)
    archGroup.traverse((o) => {
      if (o instanceof THREE.Mesh) o.raycast = () => {}
    })
    group.add(archGroup)
  }

  // Local-space occlusion AABB: the shaft footprint from this floor (y=0) up to
  // the floor above. The layer adds the group's world position before testing.
  const ur = shaftRect(layout.upShaft, ctx)
  const upShaftAABB = new THREE.Box3(
    new THREE.Vector3(ur.x, 0, ur.z),
    new THREE.Vector3(ur.x + ur.w, ctx.floorHeight, ur.z + ur.d)
  )
  return { group, upShaftAABB, wallRuns, doors }
}

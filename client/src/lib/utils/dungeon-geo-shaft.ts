/**
 * dungeon-geo-shaft.ts — shaft/rect cell-membership helpers and the stair-shaft
 * geometry. `collectShaftStairs` emits the stepped-prism staircase (plus optional
 * landings and a back wall) shared by the surface entrance, the up-shaft you
 * arrive by, and the down-shaft descending to the next floor.
 */
import * as THREE from 'three'
import type { GeoEntry } from './house-geo-utils'
import type { DungeonShaft } from '../managers/dungeonManager'
import { addBox, quadMeshBuilder } from './dungeon-geo-primitives'
import {
  SLAB_THICKNESS,
  LANDING_CELLS,
  STEP_RISE,
  DUNGEON_FLOOR_UV_SCALE,
  DUNGEON_FLOOR_TEXTURE_IDX,
  DUNGEON_WALL_TEXTURE_IDX,
  type DungeonGeoCtx,
} from './dungeon-geo-constants'

export function shaftRect(shaft: DungeonShaft, ctx: DungeonGeoCtx) {
  return shaft.alongZ
    ? { x: shaft.x, z: shaft.z, w: ctx.shaftW, d: ctx.shaftLen }
    : { x: shaft.x, z: shaft.z, w: ctx.shaftLen, d: ctx.shaftW }
}

/** Half-open cell membership: is cell (x, z) inside rect r ({x, z, w, d})?
 *  Shared by shaft and room tests; mirrors the Rust convention. */
export function rectContains(
  r: { x: number; z: number; w: number; d: number },
  x: number,
  z: number
): boolean {
  return x >= r.x && x < r.x + r.w && z >= r.z && z < r.z + r.d
}

export function shaftContains(
  shaft: DungeonShaft,
  ctx: DungeonGeoCtx,
  x: number,
  z: number
): boolean {
  return rectContains(shaftRect(shaft, ctx), x, z)
}

/** Cell at run position i (0 = entry/shallow end), lateral offset wOff. */
export function shaftStepCell(
  shaft: DungeonShaft,
  ctx: DungeonGeoCtx,
  i: number,
  wOff: number
): { x: number; z: number } {
  const run = shaft.reversed ? ctx.shaftLen - 1 - i : i
  return shaft.alongZ
    ? { x: shaft.x + wOff, z: shaft.z + run }
    : { x: shaft.x + run, z: shaft.z + wOff }
}

/**
 * Stair geometry for one shaft, local to the floor group. `topY`/`bottomY`
 * are local Y of the shallow and deep landings. Adds the steps plus flat
 * landing platforms at both ends (the far landing belongs to the
 * neighbouring floor's slab, which isn't rendered — without a platform
 * you'd stand on visual void before the floor switch).
 */
export function collectShaftStairs(
  entries: GeoEntry[],
  shaft: DungeonShaft,
  ctx: DungeonGeoCtx,
  topY: number,
  bottomY: number,
  includeTopLanding: boolean,
  includeBottomLanding: boolean,
  includeWall = true
) {
  const rise = topY - bottomY
  const runStart = LANDING_CELLS
  const runLen = ctx.shaftLen - LANDING_CELLS * 2
  const stepCount = Math.max(1, Math.round(rise / STEP_RISE))
  const stepRise = rise / stepCount
  const stepDepth = runLen / stepCount

  // Run-axis basis: position of run coordinate t (cells from entry end),
  // lateral center of the shaft.
  const r = shaftRect(shaft, ctx)
  const latCenter = shaft.alongZ ? r.x + r.w / 2 : r.z + r.d / 2
  const runAt = (t: number) => {
    const raw = shaft.reversed ? ctx.shaftLen - t : t
    return (shaft.alongZ ? r.z : r.x) + raw
  }
  const addRunBox = (t0: number, t1: number, h: number, cy: number) => {
    const a = runAt(t0)
    const b = runAt(t1)
    const runC = (a + b) / 2
    const runLenAbs = Math.abs(b - a)
    if (shaft.alongZ) {
      addBox(
        entries,
        DUNGEON_FLOOR_TEXTURE_IDX,
        ctx.shaftW,
        h,
        runLenAbs,
        latCenter,
        cy,
        runC,
        DUNGEON_FLOOR_UV_SCALE
      )
    } else {
      addBox(
        entries,
        DUNGEON_FLOOR_TEXTURE_IDX,
        runLenAbs,
        h,
        ctx.shaftW,
        runC,
        cy,
        latCenter,
        DUNGEON_FLOOR_UV_SCALE
      )
    }
  }

  if (includeTopLanding) {
    addRunBox(0, LANDING_CELLS, SLAB_THICKNESS, topY - SLAB_THICKNESS / 2)
  }

  // Steps as a single watertight solid (a stepped prism) rather than one
  // closed box per tread. Stacked boxes share internal faces that are hidden
  // when opaque but show through once the up-shaft fades to a ghost; a single
  // hull keeps only the outer surface (treads, risers, end faces, the two
  // stepped side profiles, and a flat underside). Opaque look is unchanged.
  {
    const w = ctx.shaftW
    const hw = w / 2
    const endU = runStart + stepCount * stepDepth
    const treadY = (i: number) => topY - (i + 0.5) * stepRise
    const tAt = (i: number) => runStart + i * stepDepth
    // Local point for run-coordinate t, height y, lateral offset latOff.
    const pt = (t: number, y: number, latOff: number) => {
      const run = runAt(t)
      return shaft.alongZ
        ? new THREE.Vector3(latCenter + latOff, y, run)
        : new THREE.Vector3(run, y, latCenter + latOff)
    }

    const { addQuad, finish } = quadMeshBuilder(DUNGEON_FLOOR_UV_SCALE)

    // Run-axis world direction (+t) and lateral axis, accounting for reversed.
    const sgn = shaft.reversed ? -1 : 1
    const runDir = shaft.alongZ
      ? new THREE.Vector3(0, 0, sgn)
      : new THREE.Vector3(sgn, 0, 0)
    const minusRun = runDir.clone().negate()
    const latAxis = shaft.alongZ
      ? new THREE.Vector3(1, 0, 0)
      : new THREE.Vector3(0, 0, 1)
    const downN = new THREE.Vector3(0, -1, 0)
    const upN = new THREE.Vector3(0, 1, 0)

    for (let i = 0; i < stepCount; i++) {
      const ty = treadY(i)
      // Stepped side profile on both lateral faces (one rectangle per tread).
      for (const s of [-1, 1] as const) {
        const n = latAxis.clone().multiplyScalar(s)
        addQuad(
          pt(tAt(i), bottomY, s * hw),
          pt(tAt(i + 1), bottomY, s * hw),
          pt(tAt(i + 1), ty, s * hw),
          pt(tAt(i), ty, s * hw),
          n
        )
      }
      // Tread (top).
      addQuad(
        pt(tAt(i), ty, -hw),
        pt(tAt(i + 1), ty, -hw),
        pt(tAt(i + 1), ty, hw),
        pt(tAt(i), ty, hw),
        upN
      )
      // Riser to the next (lower) tread, facing down-run.
      if (i < stepCount - 1) {
        const tb = tAt(i + 1)
        addQuad(
          pt(tb, treadY(i + 1), -hw),
          pt(tb, treadY(i + 1), hw),
          pt(tb, ty, hw),
          pt(tb, ty, -hw),
          runDir
        )
      }
    }
    // Underside (flat at the bottom landing).
    addQuad(
      pt(runStart, bottomY, -hw),
      pt(endU, bottomY, -hw),
      pt(endU, bottomY, hw),
      pt(runStart, bottomY, hw),
      downN
    )
    // Shallow-end face (under the top landing) and deep-end face.
    addQuad(
      pt(runStart, bottomY, -hw),
      pt(runStart, bottomY, hw),
      pt(runStart, treadY(0), hw),
      pt(runStart, treadY(0), -hw),
      minusRun
    )
    addQuad(
      pt(endU, bottomY, -hw),
      pt(endU, bottomY, hw),
      pt(endU, treadY(stepCount - 1), hw),
      pt(endU, treadY(stepCount - 1), -hw),
      runDir
    )

    finish(entries, DUNGEON_FLOOR_TEXTURE_IDX)
  }

  if (includeBottomLanding) {
    addRunBox(
      ctx.shaftLen - LANDING_CELLS,
      ctx.shaftLen,
      SLAB_THICKNESS,
      bottomY - SLAB_THICKNESS / 2
    )
  }

  // Shaft side walls (back-facing side only, camera rule as for walls):
  // along-Z shafts keep the east side (faces west), along-X the north
  // side (faces south). Vertical span runs from the slab underside
  // (topY − SLAB_THICKNESS; the wall sits inside the flanking cell's slab
  // footprint, so reaching topY would z-fight the slab) down to the floor
  // below (bottomY) — it does NOT rise to the current floor's wall/ceiling
  // height. Skipped for the surface entrance, which supplies its own
  // non-protruding pit walls.
  if (includeWall) {
    const wallTex = DUNGEON_WALL_TEXTURE_IDX
    const wallH = topY - SLAB_THICKNESS - bottomY
    const wallCy = bottomY + wallH / 2
    if (shaft.alongZ) {
      addBox(
        entries,
        wallTex,
        0.1,
        wallH,
        r.d,
        r.x + r.w + 0.05,
        wallCy,
        r.z + r.d / 2
      )
    } else {
      addBox(
        entries,
        wallTex,
        r.w,
        wallH,
        0.1,
        r.x + r.w / 2,
        wallCy,
        r.z - 0.05
      )
    }
  }
}

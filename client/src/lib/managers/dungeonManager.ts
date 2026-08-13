/**
 * dungeonManager.ts — client-side dungeon state singleton.
 *
 * Layouts come from the shared wasm generator (same code the server runs
 * natively), so entering a dungeon never downloads geometry: the manager
 * generates layouts from the entrance id, registers the passability entry
 * in the shared wasm cache (movement collision, click-to-move A* and
 * monster pathing then work unchanged), and answers height queries —
 * including the stair-shaft ramps that let the player physically walk
 * between floors. Module-level singleton like bridgeManager so
 * player-physics can consult it without prop drilling.
 */
import { get } from 'svelte/store'
import {
  dungeon_layout,
  dungeon_constants,
  dungeon_add_passability,
  dungeon_remove_passability,
  dungeon_passability_floor_cells,
  dungeon_rebuild_floor,
  dungeon_interior_doors,
  dungeon_floor_height_at,
  dungeon_entrance_ramp_height_at,
  dungeon_shaft_run_pos,
  passability_start_floor_at,
} from '../wasm/onlinerpg_shared'
import {
  currentDungeonDepth,
  currentDungeonId,
  dungeonPropsResetRevision,
  dungeonPropsRevision,
} from '../stores/dungeonStore'
import { DUNGEON_ENTRANCES, type DungeonEntranceDef } from '../data/dungeonDefs'
import { shortestWrappedDeltaX } from '../terrain/world-wrap'
import type { DungeonWall } from '../utils/dungeon-geo-constants'

export interface DungeonRoom {
  x: number
  z: number
  w: number
  d: number
}

export interface DungeonShaft {
  x: number
  z: number
  alongZ: boolean
  reversed: boolean
}

export interface DungeonSpawn {
  x: number
  z: number
  monsterType: string
  isBoss: boolean
}

/** Decorative room clutter (matches shared PropSpec). Cosmetic only — no
 *  collision, like the treasure chest. `kind` is an object-catalog id. */
export interface DungeonProp {
  x: number
  z: number
  kind: 'barrel' | 'crate' | 'chest' | 'torch_wall'
  /** Vertical stack count (1 or 2); chests are always 1. */
  stack: number
  /** Yaw in whole degrees (0..360). */
  rotation: number
}

/** A pending click-to-interact (break a barrel/crate, or open a chest): which
 *  prop, and the world XZ to close in on. */
export interface PendingPropBreak {
  depth: number
  propId: number
  x: number
  z: number
}

/** Depth/id under which the surface entrance door's open state is keyed in the
 *  synced door map (depth 0 is the surface; interior doors live at depth ≥1).
 *  The id is wire protocol — the server rejects any other id at depth 0; keep
 *  it in step with `ENTRANCE_DOOR_ID` in shared/src/dungeon/doors.rs. */
export const ENTRANCE_DOOR_DEPTH = 0
export const ENTRANCE_DOOR_ID = 0

/** Sentinel propId for the final-floor treasure chest in the click walk-up
 *  flow. Never sent to the server — the dungeon layer routes it to
 *  OpenDungeonChest instead of OpenDungeonProp. */
export const TREASURE_CHEST_PROP_ID = -1

/** One interior door placement from wasm `dungeon_interior_doors`. `wall` is
 *  0/1/2/3 for the room's N/E/S/W wall; `doorId` is the toggle-packet id. */
export interface InteriorDoorSpec {
  wall: DungeonWall
  lat0: number
  len: number
  wallLine: number
  doorId: number
}

export interface DungeonFloorLayout {
  depth: number
  rooms: DungeonRoom[]
  carved: boolean[]
  upShaft: DungeonShaft
  /** serde Option — arrives as undefined (not null) over wasm. */
  downShaft?: DungeonShaft | null
  chest?: [number, number] | null
  spawns: DungeonSpawn[]
  props: DungeonProp[]
}

export interface DungeonConstants {
  grid: number
  floorHeight: number
  wallHeight: number
  floorIndexBase: number
  shaftW: number
  shaftLen: number
  /** Flat landing length (in cells) at each end of a stair-shaft ramp. */
  landingCells: number
  maxDepth: number
  pathMaxNodes: number
  eventDeliveryRadius: number
}

export interface DungeonEntrance {
  x: number
  y: number
  z: number
}

/** Hysteresis (in run cells) around the switch point for floor switches. */
const SWITCH_HYSTERESIS = 0.3
/** Fraction of the shaft run at which the rendered floor switches — well before
 *  the 0.5 midpoint, so a short descent already reveals (and adopts as logical)
 *  the floor below, and symmetrically a climb keeps the lower floor visible
 *  until near the top. Kept low so the shown floor and the logical floor (which
 *  drives click floor-resolution and pathfinding) agree early: a click then
 *  moves you within the room you see instead of routing back out the entrance. */
const DEPTH_SWITCH_FRACTION = 0.2
/** Player collision footprint radius (m) — matches player-physics. */
const PLAYER_RADIUS = 0.3
/** Entrance wall thickness (m). Single source of truth, shared with the mesh
 *  builder (buildDungeonEntranceGroup imports this) so the collision line sits
 *  on the wall's outer face — the player stops short of the *visible* wall, not
 *  its inner footprint edge. */
export const ENTRANCE_WALL_T = 0.25
/** Drop a surface-level dungeon registration beyond this distance (m). The
 *  wide band past the delivery radius keeps boundary walking from re-running
 *  registration (layouts + passability + entrance rebuild). */
const ENTRANCE_UNREGISTER_DIST = 80

/** Shared immutable empty set returned for floors with no broken props. */
const EMPTY_BROKEN: ReadonlySet<number> = new Set<number>()

/** True if `prev` holds any member missing from `next` (the set shrank). */
function hasRemovedMember(
  prev: ReadonlySet<number>,
  next: ReadonlySet<number>
): boolean {
  for (const id of prev) if (!next.has(id)) return true
  return false
}

let consts: DungeonConstants | null = null

function constants(): DungeonConstants {
  if (!consts) consts = dungeon_constants() as DungeonConstants
  return consts
}

/** World min-corner of an entrance's cell grid (matches shared dungeon_origin). */
export function dungeonOriginX(e: { x: number }): number {
  return Math.floor(e.x) - constants().grid / 2
}

export function dungeonOriginZ(e: { z: number }): number {
  return Math.floor(e.z) - constants().grid / 2
}

/** World Y of an entrance's floor at `depth` (matches shared floor_world_y). */
export function dungeonFloorY(e: { y: number }, depth: number): number {
  return e.y - depth * constants().floorHeight
}

/** World-space center of a grid cell on one of an entrance's floors. */
export function dungeonCellCenter(
  e: { x: number; y: number; z: number },
  depth: number,
  cell: { x: number; z: number }
) {
  return {
    x: dungeonOriginX(e) + cell.x + 0.5,
    y: dungeonFloorY(e, depth),
    z: dungeonOriginZ(e) + cell.z + 0.5,
  }
}

/** Registry entrance whose grid footprint covers the position (shared
 *  `footprint_contains`: the server decides underground-ness by this test). */
export function entranceCovering(
  x: number,
  z: number
): DungeonEntranceDef | undefined {
  const grid = constants().grid
  return DUNGEON_ENTRANCES.find((e) => {
    const ox = dungeonOriginX(e)
    const oz = dungeonOriginZ(e)
    return x >= ox && x < ox + grid && z >= oz && z < oz + grid
  })
}

export interface DungeonRect {
  minX: number
  minZ: number
  maxX: number
  maxZ: number
}

/** One floor's passability edge bits + world placement (debug overlay). */
export interface DungeonFloorCells {
  /** World min-corner X (= originX). */
  originX: number
  originZ: number
  width: number
  depth: number
  /** World Y of the floor. */
  yBase: number
  /** Per-cell edge bitmask (N=1, E=2, S=4, W=8), indexed [gx + gz*width]. */
  cells: number[]
}

/**
 * Covered run-range of a shaft's surface opening, along the run axis: the
 * inset from the shaft origin to the covered span's near edge, plus the span
 * length. Anchored at the entry (shallow) end — a one-cell landing gap, then
 * half the tread span toward the deep end. Shared by shaftHoleRect (terrain
 * hole) and buildDungeonEntranceGroup (parapet/roof footprint) so the two
 * never desync. Purely cosmetic geometry, but it keys off the same landing
 * length as the shared ramp profile, so it reads it from the wasm constants.
 */
export function shaftCoverRun(
  shaftLen: number,
  reversed: boolean
): { inset: number; coverLen: number } {
  const landing = constants().landingCells
  const coverLen = (shaftLen - landing * 2) / 2
  // Reversed → entry is the high-coordinate end, so the cover sits one
  // landing gap plus its own length in from the low (deep) side.
  const deepInset = landing + coverLen
  return { inset: reversed ? deepInset : landing, coverLen }
}

/**
 * World-space XZ rect of a shaft's surface opening (see shaftCoverRun for the
 * covered span). Matches the covered entrance structure
 * (buildDungeonEntranceGroup), so terrain/grass meet the parapet on every side;
 * the lower stairs stay under the terrain past the deep end.
 */
function shaftHoleRect(
  shaft: DungeonShaft,
  originX: number,
  originZ: number,
  shaftW: number,
  shaftLen: number
): DungeonRect {
  const { inset, coverLen } = shaftCoverRun(shaftLen, shaft.reversed)
  let minX: number, maxX: number, minZ: number, maxZ: number
  if (shaft.alongZ) {
    minX = shaft.x
    maxX = shaft.x + shaftW
    minZ = shaft.z + inset
    maxZ = shaft.z + inset + coverLen
  } else {
    minX = shaft.x + inset
    maxX = shaft.x + inset + coverLen
    minZ = shaft.z
    maxZ = shaft.z + shaftW
  }
  return {
    minX: originX + minX,
    minZ: originZ + minZ,
    maxX: originX + maxX,
    maxZ: originZ + maxZ,
  }
}

/** Squared distance from point (px,pz) to segment a→b. */
function distSqPointToSegment(
  px: number,
  pz: number,
  ax: number,
  az: number,
  bx: number,
  bz: number
): number {
  const dx = bx - ax
  const dz = bz - az
  const len2 = dx * dx + dz * dz
  let t = len2 > 0 ? ((px - ax) * dx + (pz - az) * dz) / len2 : 0
  t = Math.max(0, Math.min(1, t))
  const cx = ax + t * dx
  const cz = az + t * dz
  const ex = px - cx
  const ez = pz - cz
  return ex * ex + ez * ez
}

/** True when segment p1→p2 properly crosses segment p3→p4 (touching ignored). */
function segmentsCross(
  p1x: number,
  p1z: number,
  p2x: number,
  p2z: number,
  p3x: number,
  p3z: number,
  p4x: number,
  p4z: number
): boolean {
  const side = (
    ax: number,
    az: number,
    bx: number,
    bz: number,
    cx: number,
    cz: number
  ) => (bx - ax) * (cz - az) - (bz - az) * (cx - ax)
  const d1 = side(p3x, p3z, p4x, p4z, p1x, p1z)
  const d2 = side(p3x, p3z, p4x, p4z, p2x, p2z)
  const d3 = side(p1x, p1z, p2x, p2z, p3x, p3z)
  const d4 = side(p1x, p1z, p2x, p2z, p4x, p4z)
  return (
    ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) &&
    ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))
  )
}

/** Whether stepping from→to is blocked by the wall/door segment a→b: it crosses
 *  the line outright (anti-tunneling), or it brings the player within
 *  PLAYER_RADIUS of it from farther out (so you can slide along or back away).
 *  Shared by the entrance shed and interior door collision. */
function segmentBlocksMovement(
  fromX: number,
  fromZ: number,
  toX: number,
  toZ: number,
  ax: number,
  az: number,
  bx: number,
  bz: number
): boolean {
  if (segmentsCross(fromX, fromZ, toX, toZ, ax, az, bx, bz)) return true
  const r2 = PLAYER_RADIUS * PLAYER_RADIUS
  const dTo = distSqPointToSegment(toX, toZ, ax, az, bx, bz)
  const dFrom = distSqPointToSegment(fromX, fromZ, ax, az, bx, bz)
  return dTo < r2 && dFrom >= r2
}

class DungeonManager {
  private id: string | null = null
  private entrance: DungeonEntrance | null = null
  private layouts: DungeonFloorLayout[] = []
  /** Cached surface-opening rects per entrance id (see allEntranceHoleRects). */
  private entranceRectCache = new Map<string, DungeonRect>()
  /** Broken props per depth (indices into that floor's `props`). Server-driven;
   *  cleared on enter/exit since it's scoped to the active dungeon instance. */
  private brokenProps = new Map<number, Set<number>>()
  /** Opened chest props per depth (indices into that floor's `props`). Same
   *  lifetime/scope as `brokenProps`; drives the lid-open animation, not
   *  passability (chests stay solid when open). */
  private openedProps = new Map<number, Set<number>>()
  /** A barrel/crate the player clicked and is walking toward; the dungeon layer
   *  fires the break once the player is within range (see GameSceneDungeonLayer
   *  update). Cleared on arrival, a new movement click, or leaving the dungeon. */
  private pendingBreakState: PendingPropBreak | null = null
  /** A chest the player clicked and is walking toward; the dungeon layer sends
   *  the open once the player is within range. Same lifecycle as
   *  `pendingBreakState`. */
  private pendingOpenState: PendingPropBreak | null = null
  /** Whether this session saw the treasure chest open (DungeonChestOpened
   *  broadcast). Drives the lid pose across floor rebuilds. Kept out of
   *  `openedProps` because the chest claim is per-character per night, not
   *  an instance-wide fact like the prop sets (which server snapshots
   *  replace wholesale). */
  private treasureChestOpenedState = false
  /** Open doors by depth → set of door ids, synced from the server. depth 0 is
   *  the surface entrance door; ≥1 are interior room doors. Cleared on
   *  enter/exit; (re)populated by the snapshot + live toggle broadcasts. */
  private openDoors = new Map<number, Set<number>>()
  /** True while the last surface-frame check was inside the delivery radius;
   *  a false→true crossing queues a door-snapshot re-pull (door toggles
   *  outside the radius are never delivered). */
  private doorSyncInside = true
  /** Entrance id whose door snapshot must be re-pulled, consumed once by the
   *  render layer (takeDoorSnapshotRequest). */
  private doorSnapshotRequest: string | null = null

  get active(): boolean {
    return this.id !== null
  }

  get dungeonId(): string | null {
    return this.id
  }

  get entrancePos(): DungeonEntrance | null {
    return this.entrance
  }

  get floors(): DungeonFloorLayout[] {
    return this.layouts
  }

  get consts(): DungeonConstants {
    return constants()
  }

  /** World min-corner of the cell grid (matches shared dungeon_origin). */
  get originX(): number {
    return dungeonOriginX(this.entrance!)
  }

  get originZ(): number {
    return dungeonOriginZ(this.entrance!)
  }

  floorY(depth: number): number {
    return dungeonFloorY(this.entrance!, depth)
  }

  /** Passability floor index for path queries at a given depth. */
  passabilityFloor(depth: number): number {
    return constants().floorIndexBase + depth - 1
  }

  /**
   * A* endpoint floor for a world position. Off the stairs this is the usual
   * closest-y-base lookup, but the shared stairwell model keys a shaft's
   * intermediate steps to its LOWER connected floor (surface=0 for the entrance
   * shaft), and A* can only enter them under that key — start (or aim) a search
   * mid-flight under the floor you are nearest and it routes the player the
   * whole way down the stairs first. Resolved in wasm, so the browser, the
   * agent-client and the server all answer it identically.
   */
  startFloorAt(x: number, z: number, y: number): number {
    return passability_start_floor_at(x, z, y)
  }

  /**
   * A* floor for a click target on one of the rendered stair shafts at `depth`,
   * or null when it lands on neither (the caller then falls back to its own
   * surface-vs-floor heuristic). The shaft membership test is what this adds
   * over `startFloorAt`.
   */
  shaftPathfindingFloorAt(
    x: number,
    z: number,
    y: number,
    depth: number
  ): number | null {
    const onShaft =
      this.shaftRunPos(depth, false, x, z) !== null ||
      this.shaftRunPos(depth, true, x, z) !== null
    return onShaft ? this.startFloorAt(x, z, y) : null
  }

  /**
   * Whether (x, z) sits on the entrance shaft's footprint (floor 1's up-shaft,
   * which connects the surface to floor 1). True even on the top landing, so a
   * player standing at the very mouth of the stairs counts as "in the dungeon
   * view" — the floor-1 room is what's shown there, so clicks should target it
   * rather than the surface, regardless of the still-zero logical depth.
   */
  isOnEntranceShaft(x: number, z: number): boolean {
    return this.shaftRunPos(1, false, x, z) !== null
  }

  /**
   * Debug: per-cell passability edge bits for the registered dungeon's floor
   * at the given passability floor level (see passabilityFloor). Null when no
   * dungeon is registered or the level isn't present.
   */
  floorPassabilityCells(floorLevel: number): DungeonFloorCells | null {
    if (!this.id) return null
    return dungeon_passability_floor_cells(
      this.id,
      floorLevel
    ) as DungeonFloorCells | null
  }

  layoutAt(depth: number): DungeonFloorLayout | null {
    return this.layouts[depth - 1] ?? null
  }

  /** Interior-door placements for one floor, from the shared wasm scan. */
  interiorDoorsAt(depth: number): InteriorDoorSpec[] {
    if (!this.id) return []
    return dungeon_interior_doors(this.id, depth) as InteriorDoorSpec[]
  }

  /**
   * Generate layouts and register passability for a dungeon. Idempotent
   * per entrance id.
   */
  enter(id: string, entrance: DungeonEntrance) {
    if (this.id === id) return
    if (this.id) this.exit()
    this.layouts = dungeon_layout(id) as DungeonFloorLayout[]
    dungeon_add_passability(id, entrance.x, entrance.y, entrance.z)
    this.brokenProps.clear()
    this.openedProps.clear()
    this.treasureChestOpenedState = false
    this.openDoors.clear()
    this.doorSyncInside = true
    this.doorSnapshotRequest = null
    this.id = id
    this.entrance = entrance
    currentDungeonId.set(id)
    // Doors start shut (openDoors cleared above); the snapshot reply corrects them.
  }

  /** Drop dungeon state (passability included). Depth resets to surface. */
  exit() {
    if (this.id) dungeon_remove_passability(this.id)
    this.id = null
    this.entrance = null
    this.layouts = []
    this.brokenProps.clear()
    this.openedProps.clear()
    this.treasureChestOpenedState = false
    this.openDoors.clear()
    this.doorSyncInside = true
    this.doorSnapshotRequest = null
    this.pendingBreakState = null
    this.pendingOpenState = null
    currentDungeonId.set(null)
    currentDungeonDepth.set(0)
  }

  /** Prop indices broken on a floor (empty set when none/unknown). */
  brokenPropsForDepth(depth: number): ReadonlySet<number> {
    return this.brokenProps.get(depth) ?? EMPTY_BROKEN
  }

  /** Prop indices (chests) opened on a floor (empty set when none/unknown). */
  openedPropsForDepth(depth: number): ReadonlySet<number> {
    return this.openedProps.get(depth) ?? EMPTY_BROKEN
  }

  /** Whether a specific prop is already known opened (skip a redundant walk-up). */
  isPropOpened(depth: number, propId: number): boolean {
    return this.openedProps.get(depth)?.has(propId) ?? false
  }

  /**
   * Replace the broken + opened prop sets for a floor from the server's
   * on-entry snapshot, refresh that floor's passability (broken only) and
   * signal the render layer.
   */
  setPropsState(
    entranceId: string,
    depth: number,
    broken: number[],
    opened: number[]
  ) {
    if (entranceId !== this.id) return
    const previousBroken = this.brokenProps.get(depth) ?? EMPTY_BROKEN
    const previousOpened = this.openedProps.get(depth) ?? EMPTY_BROKEN
    const nextBroken = new Set(broken)
    const nextOpened = new Set(opened)
    const removedState =
      hasRemovedMember(previousBroken, nextBroken) ||
      hasRemovedMember(previousOpened, nextOpened)
    this.brokenProps.set(depth, nextBroken)
    this.openedProps.set(depth, nextOpened)
    this.rebuildFloorPassability(depth)
    if (removedState) dungeonPropsResetRevision.update((n) => n + 1)
    dungeonPropsRevision.update((n) => n + 1)
  }

  /**
   * Record a single newly-broken prop (live break broadcast). No-op if already
   * known broken, so a re-broadcast won't thrash the render layer.
   */
  markPropBroken(entranceId: string, depth: number, propId: number) {
    if (entranceId !== this.id) return
    let set = this.brokenProps.get(depth)
    if (!set) {
      set = new Set()
      this.brokenProps.set(depth, set)
    }
    if (set.has(propId)) return
    set.add(propId)
    this.rebuildFloorPassability(depth)
    dungeonPropsRevision.update((n) => n + 1)
  }

  /**
   * Record a single newly-opened chest (live open broadcast). No-op if already
   * known open. Drives only the render layer (no passability change — the
   * chest stays solid when open).
   */
  markPropOpened(entranceId: string, depth: number, propId: number) {
    if (entranceId !== this.id) return
    let set = this.openedProps.get(depth)
    if (!set) {
      set = new Set()
      this.openedProps.set(depth, set)
    }
    if (set.has(propId)) return
    set.add(propId)
    dungeonPropsRevision.update((n) => n + 1)
  }

  get treasureChestOpened(): boolean {
    return this.treasureChestOpenedState
  }

  /** Record the treasure chest opening (live broadcast); the render layer
   *  plays the lid animation off the revision bump. */
  markTreasureChestOpened(entranceId: string) {
    if (entranceId !== this.id || this.treasureChestOpenedState) return
    this.treasureChestOpenedState = true
    dungeonPropsRevision.update((n) => n + 1)
  }

  /**
   * Rebuild a floor's wasm passability from its current dynamic state — broken
   * props (open their cells) plus shut interior doors (seal the corridor
   * mouth; the wasm side derives the segments from the layout). Both must go
   * through one call: the floor is regenerated from the layout each time, so
   * applying one set alone would drop the other.
   */
  private rebuildFloorPassability(depth: number) {
    if (!this.id || depth < 1) return
    const broken = Uint32Array.from(this.brokenProps.get(depth) ?? [])
    const open = Uint32Array.from(this.openDoors.get(depth) ?? [])
    dungeon_rebuild_floor(this.id, depth, broken, open)
  }

  get pendingBreak(): PendingPropBreak | null {
    return this.pendingBreakState
  }

  setPendingBreak(pending: PendingPropBreak) {
    this.pendingBreakState = pending
  }

  clearPendingBreak() {
    this.pendingBreakState = null
  }

  get pendingOpen(): PendingPropBreak | null {
    return this.pendingOpenState
  }

  setPendingOpen(pending: PendingPropBreak) {
    this.pendingOpenState = pending
  }

  clearPendingOpen() {
    this.pendingOpenState = null
  }

  setDepth(depth: number) {
    currentDungeonDepth.set(depth)
  }

  /** Grid cell of a shaft's entry landing (shallower floor). */
  shaftEntryCell(shaft: DungeonShaft): { x: number; z: number } {
    const run = shaft.reversed ? constants().shaftLen - 1 : 0
    return shaft.alongZ
      ? { x: shaft.x, z: shaft.z + run }
      : { x: shaft.x + run, z: shaft.z }
  }

  /** Grid cell of a shaft's exit landing (deeper floor). */
  shaftExitCell(shaft: DungeonShaft): { x: number; z: number } {
    const run = shaft.reversed ? 0 : constants().shaftLen - 1
    return shaft.alongZ
      ? { x: shaft.x, z: shaft.z + run }
      : { x: shaft.x + run, z: shaft.z }
  }

  /** World-space center of a grid cell at a given depth's floor Y. */
  cellCenter(depth: number, cell: { x: number; z: number }) {
    return dungeonCellCenter(this.entrance!, depth, cell)
  }

  /**
   * World-space XZ rect of the currently-registered dungeon's surface
   * opening. Used by the terrain shader to discard fragments there, opening
   * up the descending stairs. Null when no dungeon is registered.
   */
  entranceHoleRect(): DungeonRect | null {
    const first = this.layoutAt(1)
    if (!first || !this.entrance) return null
    const { shaftW, shaftLen } = constants()
    return shaftHoleRect(
      first.upShaft,
      this.originX,
      this.originZ,
      shaftW,
      shaftLen
    )
  }

  /**
   * Surface entrance structure walls block movement (the player would otherwise
   * walk straight into the open shaft). The covered footprint is a little shed
   * around the stair hole with stone walls on its two run-axis sides and far
   * (deep) end, and a doorway at the entry end; this seals all four so the only
   * way in is the doorway — and that, only while the door is open.
   *
   * Client-only collision at surface Y (the dungeon's own stairwell walls are
   * registered below the surface so they don't catch players walking above).
   * Never blocks underground (depth ≥ 1) or with no dungeon registered.
   *
   * Per edge, mirrors housing's circle test so the player stops ~PLAYER_RADIUS
   * short of the wall instead of burying into it: blocked if the step crosses
   * the wall line outright (anti-tunneling), or if it brings the player within
   * PLAYER_RADIUS of the line from farther out (the escape clause — already
   * inside the radius doesn't block, so you can slide along it or back away).
   */
  entranceBlocksMovement(
    fromX: number,
    fromZ: number,
    toX: number,
    toZ: number
  ): boolean {
    if (!this.active) return false
    if (get(currentDungeonDepth) !== 0) return false
    const first = this.layoutAt(1)
    if (!first) return false

    const shaft = first.upShaft
    const { shaftW, shaftLen } = constants()
    const { inset, coverLen } = shaftCoverRun(shaftLen, shaft.reversed)

    // Covered footprint rect in world space (matches buildDungeonEntranceGroup).
    let x0: number, x1: number, z0: number, z1: number
    if (shaft.alongZ) {
      x0 = this.originX + shaft.x
      x1 = x0 + shaftW
      z0 = this.originZ + shaft.z + inset
      z1 = z0 + coverLen
    } else {
      x0 = this.originX + shaft.x + inset
      x1 = x0 + coverLen
      z0 = this.originZ + shaft.z
      z1 = z0 + shaftW
    }

    // Wall edges: the two run-axis sides + the far (deep) end are always solid;
    // the entry end is the doorway, solid only while the door is shut. Walls use
    // their outer faces (footprint inflated by T) so the 0.3m margin keeps the
    // player off the *visible* wall; the doorway stays on the door panel plane.
    const T = ENTRANCE_WALL_T
    const nonrev = !shaft.reversed
    const doorClosed = !this.isDoorOpen(ENTRANCE_DOOR_DEPTH, ENTRANCE_DOOR_ID)
    const edges: [number, number, number, number][] = []
    if (shaft.alongZ) {
      const xa = x0 - T
      const xb = x1 + T
      const farZ = nonrev ? z1 + T : z0 - T
      const entryZ = nonrev ? z0 : z1
      const sz0 = Math.min(entryZ, farZ)
      const sz1 = Math.max(entryZ, farZ)
      edges.push([xa, sz0, xa, sz1], [xb, sz0, xb, sz1]) // sides
      edges.push([xa, farZ, xb, farZ]) // deep end
      if (doorClosed) edges.push([x0, entryZ, x1, entryZ]) // doorway
    } else {
      const za = z0 - T
      const zb = z1 + T
      const farX = nonrev ? x1 + T : x0 - T
      const entryX = nonrev ? x0 : x1
      const sx0 = Math.min(entryX, farX)
      const sx1 = Math.max(entryX, farX)
      edges.push([sx0, za, sx1, za], [sx0, zb, sx1, zb]) // sides
      edges.push([farX, za, farX, zb]) // deep end
      if (doorClosed) edges.push([entryX, z0, entryX, z1]) // doorway
    }

    for (const [ax, az, bx, bz] of edges) {
      if (segmentBlocksMovement(fromX, fromZ, toX, toZ, ax, az, bx, bz))
        return true
    }
    return false
  }

  /** Whether a door (depth, id) is currently open (default shut). */
  isDoorOpen(depth: number, doorId: number): boolean {
    return this.openDoors.get(depth)?.has(doorId) ?? false
  }

  private setDoorOpen(depth: number, doorId: number, open: boolean) {
    let set = this.openDoors.get(depth)
    if (!set) {
      set = new Set()
      this.openDoors.set(depth, set)
    }
    if (open) set.add(doorId)
    else set.delete(doorId)
  }

  /** Apply a server door-toggle broadcast (entrance at depth 0, or an interior
   *  room door). The rendered swing + collision both read the door map. */
  applyDoorToggle(
    entranceId: string,
    depth: number,
    doorId: number,
    isOpen: boolean
  ) {
    if (entranceId !== this.id) return
    this.setDoorOpen(depth, doorId, isOpen)
    // Reflect the new open/shut state into pathfinding so monster AI stops
    // walking through (or now routes around) the door.
    this.rebuildFloorPassability(depth)
  }

  /** Replace the whole open-door set from the server snapshot (reply to a
   *  RequestDungeonDoors, and pushed on every floor entry). */
  applyDoorsSnapshot(entranceId: string, doors: [number, number][]) {
    if (entranceId !== this.id) return
    this.openDoors.clear()
    for (const [depth, doorId] of doors) this.setDoorOpen(depth, doorId, true)
    // The snapshot may touch several depths at once.
    for (let depth = 1; depth <= this.layouts.length; depth++)
      this.rebuildFloorPassability(depth)
  }

  /**
   * Surface-opening rects for *all* registry dungeons, independent of
   * proximity registration. Layouts are generated once (deterministic from
   * the entrance id) and the resulting rects cached. Used to suppress grass
   * over entrances so the opening is always visible, even before the dungeon
   * registers.
   */
  allEntranceHoleRects(): DungeonRect[] {
    const out: DungeonRect[] = []
    for (const e of DUNGEON_ENTRANCES) {
      let rect = this.entranceRectCache.get(e.id)
      if (!rect) {
        // Defensive: if wasm isn't ready yet, skip this entrance without
        // caching so a later call retries (rather than throwing and aborting
        // the grass tile load).
        try {
          const { shaftW, shaftLen } = constants()
          const layouts = dungeon_layout(e.id) as DungeonFloorLayout[]
          const first = layouts[0]
          if (!first) continue
          rect = shaftHoleRect(
            first.upShaft,
            dungeonOriginX(e),
            dungeonOriginZ(e),
            shaftW,
            shaftLen
          )
          this.entranceRectCache.set(e.id, rect)
        } catch {
          continue
        }
      }
      out.push(rect)
    }
    return out
  }

  /**
   * Run position along one of floor `depth`'s shafts, in [0, shaftLen) from the
   * entry (shallow) end, or null off the footprint. `down` picks the shaft
   * descending to `depth + 1` instead of the one arriving at `depth`.
   */
  private shaftRunPos(
    depth: number,
    down: boolean,
    x: number,
    z: number
  ): number | null {
    if (!this.id) return null
    const t = dungeon_shaft_run_pos(this.id, depth, down, x, z)
    return Number.isNaN(t) ? null : t
  }

  /**
   * Ground height on a specific dungeon floor (stair-shaft ramps
   * included), independent of the local player's depth — used for
   * monsters and other entities on arbitrary floors. Null when inactive.
   *
   * The ramp profile itself lives in the shared crate (`dungeon::stairs`):
   * the server accepts a declared dungeon floor only when the reported Y is
   * within its tolerance band, so every client has to compute the same height.
   */
  floorHeightAt(depth: number, x: number, z: number): number | null {
    if (!this.id) return null
    const y = dungeon_floor_height_at(this.id, depth, x, z)
    return Number.isNaN(y) ? null : y
  }

  /**
   * Surface entrance ramp height, or null when (x, z) is off the shaft.
   * Independent of any observer's depth, unlike `sampleHeightAt`.
   */
  entranceRampHeightAt(x: number, z: number): number | null {
    if (!this.id) return null
    const y = dungeon_entrance_ramp_height_at(this.id, x, z)
    return Number.isNaN(y) ? null : y
  }

  /**
   * Ground height for the local player while the dungeon is active, or
   * null when terrain should be used instead (surface, outside shafts).
   */
  sampleHeightAt(x: number, z: number): number | null {
    if (!this.active) return null
    const depth = get(currentDungeonDepth)
    if (depth === 0) return this.entranceRampHeightAt(x, z)
    return this.floorHeightAt(depth, x, z)
  }

  /**
   * Register/unregister registry dungeons by proximity so the entrance
   * structure and passability exist before the player reaches the stairs.
   * Registration and the door-snapshot re-pull share the server's delivery
   * radius (shared EVENT_DELIVERY_RADIUS via dungeon_constants()): while
   * registered on the surface, each crossing back inside it queues a re-pull,
   * since door toggles outside the radius are never delivered.
   */
  private updateAutoRegister(x: number, z: number) {
    const r = constants().eventDeliveryRadius
    if (!this.active) {
      for (const e of DUNGEON_ENTRANCES) {
        const dx = shortestWrappedDeltaX(e.x, x)
        const dz = z - e.z
        if (dx * dx + dz * dz < r * r) {
          this.enter(e.id, { x: e.x, y: e.y, z: e.z })
          return
        }
      }
      return
    }
    if (get(currentDungeonDepth) === 0 && this.entrance) {
      const dx = shortestWrappedDeltaX(this.entrance.x, x)
      const dz = z - this.entrance.z
      const d2 = dx * dx + dz * dz
      const inside = d2 < r * r
      if (inside && !this.doorSyncInside) this.doorSnapshotRequest = this.id
      this.doorSyncInside = inside
      if (d2 > ENTRANCE_UNREGISTER_DIST * ENTRANCE_UNREGISTER_DIST) {
        this.exit()
      }
    }
  }

  /**
   * Adopt a server-driven floor change (teleport/respawn): activate the
   * covering registry dungeon when needed and set the depth.
   */
  syncFromFloorLevel(floorLevel: number, x: number, z: number) {
    if (floorLevel >= 0) {
      if (this.active) {
        currentDungeonDepth.set(0)
        // Surfacing via a server sync (respawn/teleport) shuts the entrance
        // door locally, mirroring enter()/exit() (the snapshot reply corrects
        // it to the authoritative state if someone left it open).
        this.setDoorOpen(ENTRANCE_DOOR_DEPTH, ENTRANCE_DOOR_ID, false)
      }
      return
    }
    const covering = entranceCovering(x, z)
    if (!covering) return
    // Teleports can land in a different dungeon; enter() no-ops on the same id.
    this.enter(covering.id, { x: covering.x, y: covering.y, z: covering.z })
    currentDungeonDepth.set(-floorLevel)
  }

  /** Entrance id needing a door-snapshot re-pull (the player crossed back
   *  into delivery range); cleared on read. */
  takeDoorSnapshotRequest(): string | null {
    const id = this.doorSnapshotRequest
    this.doorSnapshotRequest = null
    return id
  }

  /**
   * Per-frame: switch the current depth when the player walks past the shaft
   * switch point (DEPTH_SWITCH_FRACTION of the run, not the midpoint). Returns
   * the new depth, or null when unchanged.
   */
  updateFromPlayerPosition(x: number, z: number): number | null {
    this.updateAutoRegister(x, z)
    if (!this.active) return null
    const depth = get(currentDungeonDepth)
    const switchPoint = constants().shaftLen * DEPTH_SWITCH_FRACTION

    if (depth === 0) {
      const t = this.shaftRunPos(1, false, x, z)
      if (t !== null && t > switchPoint + SWITCH_HYSTERESIS) {
        currentDungeonDepth.set(1)
        return 1
      }
      return null
    }

    const tUp = this.shaftRunPos(depth, false, x, z)
    if (tUp !== null && tUp < switchPoint - SWITCH_HYSTERESIS) {
      const next = depth - 1
      currentDungeonDepth.set(next)
      return next
    }
    const tDown = this.shaftRunPos(depth, true, x, z)
    if (tDown !== null && tDown > switchPoint + SWITCH_HYSTERESIS) {
      const next = depth + 1
      currentDungeonDepth.set(next)
      return next
    }
    return null
  }
}

export const dungeonManager = new DungeonManager()

/**
 * dungeon-geo-constants.ts — shared texture indices, dimensional constants,
 * the build context and the wall-side enums for the dungeon mesh builders.
 * Leaf module: only depends on the housing texture table.
 */
import { HOUSING_TEXTURES } from './house-geo-utils'

export const DUNGEON_WALL_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'housing/medieval_blocks_03_1k'
)
/** Rock-wall-10 texture for the *connecting corridors* — distinct from the
 *  medieval-stone room walls (DUNGEON_WALL_TEXTURE_IDX). A cell gets this
 *  texture when it is carved floor that lies in no room (and no shaft). */
export const DUNGEON_CORRIDOR_WALL_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'dungeon/rock_wall_10_1k'
)
/** Mossy plaster for the *surface* entrance building walls — distinct from the
 *  underground stone walls (DUNGEON_WALL_TEXTURE_IDX). */
export const DUNGEON_ENTRANCE_WALL_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'housing/worn_mossy_plasterwall_1k'
)
export const DUNGEON_FLOOR_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'housing/grey_stone_path_1k'
)
export const DUNGEON_VOID_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.label === 'Void'
)
/** Grey roof tiles for the surface entrance roof. */
export const DUNGEON_CEILING_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'housing/grey_roof_tiles_02_1k'
)
/** Stone blocks for the decorative entrance corner pillars (accent against the
 *  mossy-plaster entrance walls). */
export const DUNGEON_PILLAR_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'housing/medieval_blocks_03_1k'
)
/** Wooden garage-door texture for the entrance door (mapped 0→1 across it). */
export const DUNGEON_DOOR_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'dungeon/wooden_garage_door_1k'
)
/** Rusty grid for doors that take a floor key (doc/DUNGEON_REWARD.md). */
export const DUNGEON_LOCKED_DOOR_TEXTURE_IDX = HOUSING_TEXTURES.findIndex(
  (e) => e.glb === 'dungeon/rusty_metal_grid_1k'
)

// Dungeon stone needs no explicit shadowSide: the housing materials are
// `side: FrontSide`, so three.js derives BackSide, and each solid's own thickness
// (0.15m slab, 0.1m walls) self-biases away acne. (FrontSide was tried to anchor
// contact shadows but acned the walls.) BackSide records the far face, so where a
// caster's bottom sits exactly at floor level the shadow peter-pans (no contact
// darkening); lifting the caster a hair pulls that recorded depth above the floor.
/** Vertical lift (m) off the floor for floor-contacting dungeon casters (stair
 *  prisms, back walls). The base seam is sub-texel under PCF, and behind a back
 *  wall is uncarved rock, so the gap reveals only void, never lit floor. */
export const SHADOW_CONTACT_LIFT = 0.02

/** Name of the up-shaft stairs sub-group inside a floor group; the dungeon
 *  layer looks it up to fade it to a ghost when it occludes the player. */
export const UP_SHAFT_GROUP_NAME = 'upShaftStairs'

export const SLAB_THICKNESS = 0.15
/** Flat landing cells at shaft ends — must match dungeonManager.rampY. */
export const LANDING_CELLS = 1.0
export const STEP_RISE = 0.25
/** UV scale for the dungeon floor/stairs texture: <1 enlarges the stone pattern
 *  (one repeat spans 1/scale metres) to cut the visible tiling. Dungeon-only —
 *  housing bakes its own UVs, so the shared texture is unaffected there. */
export const DUNGEON_FLOOR_UV_SCALE = 0.5

/** Thickness of a dungeon wall-run mesh; its plane sits half this past the cell
 *  face it guards (a north wall at z − HALF, an east wall at x + 1 + HALF). */
export const WALL_THICKNESS = 0.1
export const WALL_HALF_THICKNESS = WALL_THICKNESS / 2

export interface DungeonGeoCtx {
  grid: number
  /** Wall visual height (matches shared DUNGEON_WALL_HEIGHT). */
  wallHeight: number
  /** Vertical distance between floors (shared DUNGEON_FLOOR_HEIGHT). */
  floorHeight: number
  shaftW: number
  shaftLen: number
}

/** Which of a room's four walls a corridor mouth sits in. Packed into the door
 *  id, so the values double as encoding lanes (0..3). */
export const WALL_N = 0
export const WALL_E = 1
export const WALL_S = 2
export const WALL_W = 3
export type DungeonWall =
  | typeof WALL_N
  | typeof WALL_E
  | typeof WALL_S
  | typeof WALL_W

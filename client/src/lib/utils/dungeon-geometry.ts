/**
 * dungeon-geometry.ts — procedural mesh building for dungeon floors.
 *
 * Follows the housing pattern: collect GeoEntry quads/boxes per texture
 * index, merge into one mesh per texture (addMergedMeshes), reuse the
 * shared housing materials so no new WebGPU pipelines are compiled.
 *
 * Conventions (must mirror shared/src/dungeon):
 * - Group origin sits at (originX, floorY(depth), originZ); all geometry
 *   is local. Local y=0 is this floor's walking surface.
 * - No ceiling on underground floors: the isometric camera looks down ~35°,
 *   any current-floor ceiling would fully occlude the player. The void reads
 *   as cave dark. (The surface entrance is the one exception — it carries a
 *   gravel roof, always shown at depth 0; the player only descends the stairs,
 *   never stands inside, so it never needs to hide.)
 * - Walls on all four sides are emitted as per-run meshes in their own
 *   sub-group; the dungeon layer fades any run to a
 *   ghost while it occludes the player from the iso camera. Camera-facing
 *   south/west runs fade often; the far north/east runs only when the layout
 *   puts them between the camera and the player. Floor slab, chest and the
 *   down-shaft stairs stay merged into the shared opaque mesh.
 * - Stair shafts render both directions per floor: the up shaft you
 *   arrived by (rising to +floorHeight) and the down shaft (descending
 *   to -floorHeight). Adjacent floors build the identical world-space
 *   boxes for the shared shaft, so switching the rendered floor at the
 *   shaft midpoint is seamless.
 *
 * This module is the public entry point: it re-exports the builders and types
 * from the sibling `dungeon-geo-*.ts` modules (split by concern) and owns the
 * `disposeDungeonGroup` lifecycle helper. Importers keep using this path.
 */
import * as THREE from 'three'

export {
  DUNGEON_WALL_TEXTURE_IDX,
  DUNGEON_CORRIDOR_WALL_TEXTURE_IDX,
  DUNGEON_ENTRANCE_WALL_TEXTURE_IDX,
  DUNGEON_FLOOR_TEXTURE_IDX,
  DUNGEON_VOID_TEXTURE_IDX,
  DUNGEON_CEILING_TEXTURE_IDX,
  DUNGEON_PILLAR_TEXTURE_IDX,
  UP_SHAFT_GROUP_NAME,
} from './dungeon-geo-constants'
export type { DungeonGeoCtx } from './dungeon-geo-constants'

export { shaftStepCell } from './dungeon-geo-shaft'

export type { DoorLeaf, InteriorDoor } from './dungeon-geo-doors'

export { buildDungeonFloorGroup } from './dungeon-geo-floor'
export type { WallRun, DungeonFloorGroup } from './dungeon-geo-floor'

export { buildDungeonEntranceGroup } from './dungeon-geo-entrance'
export type { DungeonEntranceGroup } from './dungeon-geo-entrance'

/** Dispose merged geometries (materials are shared — never disposed). */
export function disposeDungeonGroup(group: THREE.Group) {
  group.traverse((obj) => {
    if (obj instanceof THREE.Mesh) obj.geometry.dispose()
  })
}

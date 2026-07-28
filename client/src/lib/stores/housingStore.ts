import { writable } from 'svelte/store'

/** Y offset to raise entities above the house floor surface */
export const playerFloorOffset = writable(0)

/**
 * Passability floor for movement/physics: 0 = ground level (outdoors or a
 * house 1F), 1 = 2F, and transiently `floorIndexBase`+ for dungeon floors
 * while underground. Whether the player is indoors is `playerInsideHouseId`,
 * not a floor sentinel. Movement pre-sets this to the next A* waypoint's
 * floor before a climb starts, so it is NOT "the floor the player stands
 * on" — that is `playerVisualFloorLevel`.
 */
export const playerFloorLevel = writable(0)

/**
 * Floor the player *visually* occupies (same 0-based convention), from the
 * stairwell hysteresis alone (GameSceneHousingLayer is the only writer).
 * Unlike `playerFloorLevel` this only flips once the player has actually
 * risen. Use it for anything about where the player really stands — the wire
 * `floor_level`, the remote visibility gate, door-click floor checks — so a
 * player doesn't vanish from (or fail to act on) the floor they're still
 * standing on.
 */
export const playerVisualFloorLevel = writable(0)

/** ID of the house the player is currently inside, or null if outdoors */
export const playerInsideHouseId = writable<string | null>(null)

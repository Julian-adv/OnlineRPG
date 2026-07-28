import { writable } from 'svelte/store'

/** Y offset to raise entities above the house floor surface */
export const playerFloorOffset = writable(0)

/**
 * Floor the player stands on: 0 = ground level (outdoors or a house 1F),
 * 1 = 2F. Flips via the stairwell hysteresis once the player has actually
 * risen (GameSceneHousingLayer is the only writer). Drives the wire
 * `floor_level`, the remote visibility gate, door-click floor checks, and
 * idle passability (while path-following, `MovingStateData.floor` takes
 * over). Whether the player is indoors is `playerInsideHouseId`, not a
 * floor sentinel.
 */
export const playerVisualFloorLevel = writable(0)

/** ID of the house the player is currently inside, or null if outdoors */
export const playerInsideHouseId = writable<string | null>(null)

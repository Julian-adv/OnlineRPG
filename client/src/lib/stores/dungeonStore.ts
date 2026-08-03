import { derived, writable } from 'svelte/store'

/**
 * Dungeon depth the local player is on. 0 = surface, 1..N = floors below
 * the entrance. Kept separate from housing's playerVisualFloorLevel (0 =
 * ground level); the wire format `floor_level = -depth` is produced only at
 * the network boundary.
 */
export const currentDungeonDepth = writable(0)

/** Entrance id of the dungeon the player is in (null = none). */
export const currentDungeonId = writable<string | null>(null)

/** True while the player is below the surface (hides overworld layers). */
export const isUnderground = derived(currentDungeonDepth, (d) => d >= 1)

/**
 * Entrance ids recorded on the world map, from
 * ServerMessage::DungeonDiscoveries (full snapshot at join and after each
 * discovery — always replaced wholesale, never mutated in place).
 */
export const discoveredDungeonIds = writable<ReadonlySet<string>>(new Set())

export function resetDiscoveredDungeons() {
  discoveredDungeonIds.set(new Set())
}

/**
 * Bumped whenever the set of broken props on any floor changes (server snapshot
 * on entry, or a live break broadcast). The dungeon render layer watches this to
 * reconcile prop meshes with their broken variants; the authoritative set lives
 * on `dungeonManager.brokenPropsForDepth(depth)`.
 */
export const dungeonPropsRevision = writable(0)

/**
 * Bumped when an authoritative prop snapshot removes already-known broken/open
 * state, e.g. the debug reset command. The render layer must rebuild props in
 * that case because broken debris/open chest poses cannot be reconciled
 * backwards in place.
 */
export const dungeonPropsResetRevision = writable(0)

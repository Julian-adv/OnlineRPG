import { writable } from 'svelte/store'
import type { FishState, Position } from '../network/networkTypes'

/** The local player's current struggle round. The countdown is animated
 *  client-side per round; the authoritative deadline lives on the server. */
export type StruggleRound = {
  round: number
  totalRounds: number
  fishState: FishState
  respondWithinMs: number
  tension: number
}

/** The local player's place in the fishing loop — one value, so phase and
 *  struggle round can never disagree. `casting` covers the whole
 *  cast-through-wait stretch (only the server knows when the wait ends);
 *  `bite` is the act-now window. */
export type MyFishing =
  | { phase: 'idle' | 'casting' | 'bite' }
  | { phase: 'struggle'; struggle: StruggleRound }

export const myFishing = writable<MyFishing>({ phase: 'idle' })

export function applyStruggleTension(tension: number) {
  myFishing.update((f) =>
    f.phase === 'struggle' ? { ...f, struggle: { ...f.struggle, tension } } : f
  )
}

export type BobberState = {
  position: Position
  /** True once the fish bit — the bobber renders its dip. */
  bite: boolean
}

/** Every visible bobber, keyed by owning player id (broadcasts are
 *  radius-gated server-side, so this map is already "nearby only"). */
export const fishingBobbers = writable<Map<number, BobberState>>(new Map())

export function upsertBobber(playerId: number, position: Position) {
  fishingBobbers.update((map) => {
    const next = new Map(map)
    next.set(playerId, { position, bite: false })
    return next
  })
}

export function markBobberBite(playerId: number) {
  fishingBobbers.update((map) => {
    const existing = map.get(playerId)
    if (!existing) return map
    const next = new Map(map)
    next.set(playerId, { ...existing, bite: true })
    return next
  })
}

export function removeBobber(playerId: number) {
  fishingBobbers.update((map) => {
    if (!map.has(playerId)) return map
    const next = new Map(map)
    next.delete(playerId)
    return next
  })
}

export function resetFishingStore() {
  myFishing.set({ phase: 'idle' })
  fishingBobbers.set(new Map())
}

import { writable } from 'svelte/store'
import type { FishState, Position } from '../network/networkTypes'

/** The local player's place in the fishing loop. `casting` covers the whole
 *  cast-through-wait stretch (the client doesn't know when the wait ends —
 *  only the server does); `bite` is the act-now window; `struggle` is the
 *  round-by-round fight after the hook. */
export type FishingPhase = 'idle' | 'casting' | 'bite' | 'struggle'

export const myFishingPhase = writable<FishingPhase>('idle')

/** The local player's current struggle round (null outside a struggle).
 *  `startedAt` is client receipt time — only used to animate the countdown
 *  ring; the authoritative deadline lives on the server. */
export type StruggleRound = {
  round: number
  totalRounds: number
  fishState: FishState
  respondWithinMs: number
  tension: number
  startedAt: number
}

export const myStruggle = writable<StruggleRound | null>(null)

export function setStruggleRound(round: StruggleRound) {
  myStruggle.set(round)
}

export function applyStruggleTension(tension: number) {
  myStruggle.update((s) => (s ? { ...s, tension } : s))
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
  myFishingPhase.set('idle')
  myStruggle.set(null)
  fishingBobbers.set(new Map())
}

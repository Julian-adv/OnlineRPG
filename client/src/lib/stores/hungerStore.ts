import { writable } from 'svelte/store'

export type HungerBand = 'Normal' | 'Hungry' | 'Weak'

/** Mirrors shared/src/hunger.rs NORMAL_MIN — the server denies sprint at or below it. */
export const SPRINT_MIN_SATIATION = 300

/** The local player's hunger, pushed by the server on transitions and eating
 *  (doc/HUNGER.md). `null` until the first HungerUpdate arrives. The
 *  multipliers are server-computed; the client never re-derives the bands. */
export interface HungerSnapshot {
  satiation: number
  band: HungerBand
  moveMult: number
  attackMult: number
  carryMult: number
}

export const hungerState = writable<HungerSnapshot | null>(null)

/** Local grill cast in progress. */
export const grilling = writable(false)

export function resetHungerStore() {
  hungerState.set(null)
  grilling.set(false)
}

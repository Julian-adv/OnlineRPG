import { writable } from 'svelte/store'

export type HungerBand = 'Stuffed' | 'WellFed' | 'Hungry' | 'Weak'

/** The local player's hunger, pushed by the server on transitions and eating
 *  (doc/HUNGER.md). `null` until the first HungerUpdate arrives. The
 *  multipliers are server-computed; the client never re-derives the bands. */
export interface HungerSnapshot {
  satiation: number
  band: HungerBand
  moveMult: number
  attackMult: number
  carryMult: number
  /** Epoch ms (Date.now clock) when food poisoning ends; null when healthy. */
  poisonedUntil: number | null
}

export const hungerState = writable<HungerSnapshot | null>(null)

/** Local grill cast in progress. */
export const grilling = writable(false)

export function resetHungerStore() {
  hungerState.set(null)
  grilling.set(false)
}

import { writable } from 'svelte/store'
import type { FishState, Position } from '../network/networkTypes'

/** The local player's live fight readout, refreshed by each `FishingFight`
 *  beat (4 Hz). The simulation is server-authoritative. */
export type FightStatus = {
  fishState: FishState
  /** Line tension 0–100; the line snaps at 100. */
  tension: number
  /** Fish stamina 0–100; at 0 it goes exhausted and can be landed. */
  stamina: number
}

/** The local player's place in the fishing loop — one value, so phase and
 *  fight readout can never disagree. `casting` covers the whole
 *  cast-through-wait stretch (only the server knows when the wait ends);
 *  `bite` is the act-now window. */
export type MyFishing =
  | { phase: 'idle' | 'casting' | 'bite' }
  | { phase: 'fight'; fight: FightStatus }

export const myFishing = writable<MyFishing>({ phase: 'idle' })

/** Apply a `FishingFight` beat. Opens the fight phase from `bite` (the first
 *  beat follows the hook) but never resurrects one from `idle`/`casting` —
 *  a beat racing a `FishingEnded` must stay dead. */
export function applyFightUpdate(
  fishState: FishState,
  tension: number,
  stamina: number
) {
  myFishing.update((f) => {
    if (f.phase === 'fight' || f.phase === 'bite') {
      return { phase: 'fight', fight: { fishState, tension, stamina } }
    }
    return f
  })
}

/** A fighting fish's public readout, rendered as bobber motion and splash
 *  for everyone nearby (broadcasts carry it — agent parity). */
export type BobberFight = {
  fishState: FishState
  stamina: number
}

export type BobberState = {
  position: Position
  /** Flight time left before the cast lands: the renderer keeps the float
   *  hidden (and the line undrawn) until this many ms have elapsed. */
  landsInMs: number
  /** True once the fish bit — the bobber renders its dip. */
  bite: boolean
  /** Set while the hooked fight runs; drives splash and drag motion. */
  fight?: BobberFight
}

/** Every visible bobber, keyed by owning player id (broadcasts are
 *  radius-gated server-side, so this map is already "nearby only").
 *
 *  Reactivity contract: the store notifies only on add/remove — the 4 Hz
 *  in-fight fields (`position`, `bite`, `fight`) are mutated in place and
 *  read imperatively per frame by `FishingBobber`'s task, never from
 *  templates. That keeps a beat from re-rendering every bobber row. */
let bobbers = new Map<number, BobberState>()
export const fishingBobbers = writable<Map<number, BobberState>>(bobbers)

export function upsertBobber(
  playerId: number,
  position: Position,
  landsInMs = 0
) {
  bobbers = new Map(bobbers)
  bobbers.set(playerId, { position, landsInMs, bite: false })
  fishingBobbers.set(bobbers)
}

export function markBobberBite(playerId: number) {
  const existing = bobbers.get(playerId)
  if (existing) existing.bite = true
}

/** A fight beat moves the bobber with the fish and updates its readout. */
export function updateBobberFight(
  playerId: number,
  position: Position,
  fishState: FishState,
  stamina: number
) {
  const existing = bobbers.get(playerId)
  if (existing) {
    existing.position = position
    existing.bite = false
    existing.fight = { fishState, stamina }
  }
}

export function removeBobber(playerId: number) {
  if (!bobbers.has(playerId)) return
  bobbers = new Map(bobbers)
  bobbers.delete(playerId)
  fishingBobbers.set(bobbers)
}

export function resetFishingStore() {
  myFishing.set({ phase: 'idle' })
  bobbers = new Map()
  fishingBobbers.set(bobbers)
}

import { beforeEach, describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import {
  applyStruggleTension,
  fishingBobbers,
  markBobberBite,
  myFishing,
  removeBobber,
  resetFishingStore,
  upsertBobber,
  type StruggleRound,
} from './fishingStore'

const ID = 7

function struggleRound(overrides: Partial<StruggleRound> = {}): StruggleRound {
  return {
    round: 1,
    totalRounds: 3,
    fishState: 'pulling',
    respondWithinMs: 1200,
    tension: 50,
    ...overrides,
  }
}

describe('myFishing transitions', () => {
  beforeEach(() => {
    resetFishingStore()
  })

  it('starts idle', () => {
    expect(get(myFishing)).toEqual({ phase: 'idle' })
  })

  it('updates tension during a struggle', () => {
    myFishing.set({ phase: 'struggle', struggle: struggleRound() })

    applyStruggleTension(80)

    const state = get(myFishing)
    expect(state.phase).toBe('struggle')
    if (state.phase === 'struggle') {
      expect(state.struggle.tension).toBe(80)
      expect(state.struggle.round).toBe(1)
    }
  })

  it('ignores tension outside a struggle — a late RoundResult must not corrupt the phase', () => {
    for (const phase of ['idle', 'casting', 'bite'] as const) {
      myFishing.set({ phase })
      applyStruggleTension(80)
      expect(get(myFishing)).toEqual({ phase })
    }
  })
})

describe('fishingBobbers', () => {
  beforeEach(() => {
    resetFishingStore()
  })

  it('upserts a bobber without a bite', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 })

    expect(get(fishingBobbers).get(ID)).toEqual({
      position: { x: 1, y: 0, z: 2 },
      bite: false,
    })
  })

  it('re-casting resets a previous bite', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 })
    markBobberBite(ID)
    upsertBobber(ID, { x: 3, y: 0, z: 4 })

    expect(get(fishingBobbers).get(ID)).toEqual({
      position: { x: 3, y: 0, z: 4 },
      bite: false,
    })
  })

  it('marks a bite only for an existing bobber', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 })

    markBobberBite(ID)
    markBobberBite(99)

    const map = get(fishingBobbers)
    expect(map.get(ID)?.bite).toBe(true)
    expect(map.has(99)).toBe(false)
  })

  it('a bite for an unknown player does not touch the map — no spurious rerender', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 })
    const before = get(fishingBobbers)

    markBobberBite(99)

    expect(get(fishingBobbers)).toBe(before)
  })

  it('removes a bobber; removing an absent one keeps the same map', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 })

    removeBobber(ID)
    expect(get(fishingBobbers).size).toBe(0)

    const before = get(fishingBobbers)
    removeBobber(ID)
    expect(get(fishingBobbers)).toBe(before)
  })

  it('reset clears the phase and every bobber', () => {
    myFishing.set({ phase: 'struggle', struggle: struggleRound() })
    upsertBobber(ID, { x: 1, y: 0, z: 2 })

    resetFishingStore()

    expect(get(myFishing)).toEqual({ phase: 'idle' })
    expect(get(fishingBobbers).size).toBe(0)
  })
})

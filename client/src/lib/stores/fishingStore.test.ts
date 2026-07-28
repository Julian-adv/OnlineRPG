import { beforeEach, describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import {
  applyFightUpdate,
  fishingBobbers,
  markBobberBite,
  myFishing,
  removeBobber,
  resetFishingStore,
  updateBobberFight,
  upsertBobber,
  type FightStatus,
} from './fishingStore'

const ID = 7

function fight(overrides: Partial<FightStatus> = {}): FightStatus {
  return {
    fishState: 'running',
    tension: 50,
    stamina: 80,
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

  it('the first fight beat after the hook opens the fight phase', () => {
    myFishing.set({ phase: 'bite' })

    applyFightUpdate('running', 20, 100)

    expect(get(myFishing)).toEqual({
      phase: 'fight',
      fight: { fishState: 'running', tension: 20, stamina: 100 },
    })
  })

  it('later beats update the readout', () => {
    myFishing.set({ phase: 'fight', fight: fight() })

    applyFightUpdate('exhausted', 30, 0)

    expect(get(myFishing)).toEqual({
      phase: 'fight',
      fight: { fishState: 'exhausted', tension: 30, stamina: 0 },
    })
  })

  it('a beat racing the end must not resurrect a dead fight', () => {
    for (const phase of ['idle', 'casting'] as const) {
      myFishing.set({ phase })
      applyFightUpdate('running', 50, 50)
      expect(get(myFishing)).toEqual({ phase })
    }
  })
})

describe('fishingBobbers', () => {
  beforeEach(() => {
    resetFishingStore()
  })

  it('upserts a bobber without a bite, landing immediately by default', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 })

    expect(get(fishingBobbers).get(ID)).toEqual({
      position: { x: 1, y: 0, z: 2 },
      landsInMs: 0,
      bite: false,
    })
  })

  it('a cast carries its flight time so the float lands late', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 }, 1400)

    expect(get(fishingBobbers).get(ID)?.landsInMs).toBe(1400)
  })

  it('re-casting resets a previous bite', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 })
    markBobberBite(ID)
    upsertBobber(ID, { x: 3, y: 0, z: 4 })

    expect(get(fishingBobbers).get(ID)).toEqual({
      position: { x: 3, y: 0, z: 4 },
      landsInMs: 0,
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

  it('a fight beat moves the bobber, clears the bite, and carries the readout', () => {
    upsertBobber(ID, { x: 1, y: 0, z: 2 })
    markBobberBite(ID)

    updateBobberFight(ID, { x: 3, y: 0, z: 5 }, 'running', 70)

    expect(get(fishingBobbers).get(ID)).toEqual({
      position: { x: 3, y: 0, z: 5 },
      landsInMs: 0,
      bite: false,
      fight: { fishState: 'running', stamina: 70 },
    })
  })

  it('a fight beat for an unknown player does not touch the map', () => {
    const before = get(fishingBobbers)

    updateBobberFight(99, { x: 3, y: 0, z: 5 }, 'resting', 50)

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
    myFishing.set({ phase: 'fight', fight: fight() })
    upsertBobber(ID, { x: 1, y: 0, z: 2 })

    resetFishingStore()

    expect(get(myFishing)).toEqual({ phase: 'idle' })
    expect(get(fishingBobbers).size).toBe(0)
  })
})

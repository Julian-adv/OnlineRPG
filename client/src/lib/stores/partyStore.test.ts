import { describe, expect, it, beforeEach } from 'vitest'
import { get } from 'svelte/store'
import {
  partyPositions,
  applyPartyPositions,
  resetPartyPositions,
} from './partyStore'

// The push payload is party-wide (serialized once server-side, recipient
// included); the client-side filter here is the only self-exclusion.
const push = [
  { id: 1, x: 10, z: 20, floor_level: 0 },
  { id: 2, x: 30, z: 40, floor_level: -1 },
]

describe('applyPartyPositions', () => {
  beforeEach(() => resetPartyPositions())

  it('stores the payload minus self', () => {
    applyPartyPositions(push, 1, true)
    expect(get(partyPositions)).toEqual([push[1]])
  })

  it('keeps everyone when self is not in the payload', () => {
    applyPartyPositions(push, 99, true)
    expect(get(partyPositions)).toEqual(push)
  })

  it('drops a push that crosses a disband (no roster)', () => {
    applyPartyPositions(push, 1, false)
    expect(get(partyPositions)).toEqual([])
  })

  it('filters nothing when self is unknown', () => {
    applyPartyPositions(push, undefined, true)
    expect(get(partyPositions)).toEqual(push)
  })
})

import { describe, expect, it, beforeEach } from 'vitest'
import { get } from 'svelte/store'
import {
  partyPositions,
  partyRoster,
  applyPartyPositions,
  applyPartyVitals,
  classIconPath,
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

describe('applyPartyVitals', () => {
  const member = (id: number, hp: number) => ({
    id,
    name: `p${id}`,
    hp,
    max_hp: 10,
    class: 'knight',
  })

  beforeEach(() =>
    partyRoster.set({ leaderId: 1, members: [member(1, 10), member(2, 10)] })
  )

  it('updates matching members and keeps the rest', () => {
    applyPartyVitals([{ id: 2, hp: 3, max_hp: 12 }])
    const members = get(partyRoster)!.members
    expect(members[0]).toEqual(member(1, 10))
    expect(members[1]).toEqual({ ...member(2, 3), max_hp: 12 })
  })

  it('drops ids that left the roster', () => {
    applyPartyVitals([{ id: 99, hp: 1, max_hp: 10 }])
    expect(get(partyRoster)!.members).toEqual([member(1, 10), member(2, 10)])
  })

  it('is a no-op across a disband', () => {
    partyRoster.set(null)
    applyPartyVitals([{ id: 1, hp: 5, max_hp: 10 }])
    expect(get(partyRoster)).toBeNull()
  })
})

describe('classIconPath', () => {
  it('maps the eight web-creatable classes to their icons', () => {
    for (const cls of [
      'knight',
      'barbarian',
      'caveman',
      'valkyrie',
      'ranger',
      'priest',
      'rogue',
      'bard',
    ]) {
      expect(classIconPath(cls)).toBe(`/icons/party/class-${cls}.svg`)
    }
  })

  it('returns null for agent-only classes so the panel falls back', () => {
    for (const cls of ['samurai', 'wizard', 'tourist', 'merchant', 'guard']) {
      expect(classIconPath(cls)).toBeNull()
    }
  })
})

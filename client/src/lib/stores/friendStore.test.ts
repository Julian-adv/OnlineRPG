import { describe, expect, it, beforeEach } from 'vitest'
import { get } from 'svelte/store'
import {
  metAgo,
  applyFriendList,
  applyFriendsOnline,
  friendList,
  onlineFriends,
  pendingFriendRequests,
  resetFriendStores,
  sortFriends,
  type FriendEntry,
} from './friendStore'

const roster: FriendEntry[] = [
  { characterId: 1, name: 'alice', level: 12, class: 'knight' },
  { characterId: 2, name: 'bob', level: 8, class: 'ranger' },
]

describe('applyFriendsOnline', () => {
  beforeEach(() => resetFriendStores())

  it('is silent on the session first answer but stores presence', () => {
    const announced = applyFriendsOnline(
      [{ character_id: 1, level: 13 }],
      roster
    )
    expect(announced).toEqual([])
    expect(get(onlineFriends)).toEqual(new Map([[1, 13]]))
  })

  it('announces friends who appear in a later answer', () => {
    applyFriendsOnline([{ character_id: 1, level: 12 }], roster)
    const announced = applyFriendsOnline(
      [
        { character_id: 1, level: 12 },
        { character_id: 2, level: 9 },
      ],
      roster
    )
    expect(announced).toEqual(['bob'])
  })

  it('carries the live level, which the roster snapshot cannot', () => {
    applyFriendsOnline([{ character_id: 1, level: 12 }], roster)
    applyFriendsOnline([{ character_id: 1, level: 15 }], roster)
    expect(get(onlineFriends).get(1)).toBe(15)
  })

  it('treats absence as offline', () => {
    applyFriendsOnline([{ character_id: 1, level: 12 }], roster)
    applyFriendsOnline([], roster)
    expect(get(onlineFriends).size).toBe(0)
  })

  it('re-announces a friend who went away and came back', () => {
    applyFriendsOnline([{ character_id: 1, level: 12 }], roster)
    applyFriendsOnline([], roster)
    expect(
      applyFriendsOnline([{ character_id: 1, level: 12 }], roster)
    ).toEqual(['alice'])
  })

  it('skips ids the roster cannot name', () => {
    applyFriendsOnline([], roster)
    expect(applyFriendsOnline([{ character_id: 9, level: 1 }], roster)).toEqual(
      []
    )
  })
})

describe('applyFriendList', () => {
  beforeEach(() => resetFriendStores())

  it('clears presence when the last friend goes, since no poll will', () => {
    applyFriendList(roster)
    applyFriendsOnline([{ character_id: 1, level: 12 }], roster)
    applyFriendList([])
    expect(get(friendList)).toEqual([])
    expect(get(onlineFriends).size).toBe(0)
  })

  it('takes down a toast from someone who is now a friend', () => {
    // The reciprocal-request path forms the friendship with no answer given,
    // so nothing else would dismiss the requester's toast.
    pendingFriendRequests.set([
      { requesterId: 7, requesterName: 'alice', offeredAt: 0 },
      { requesterId: 8, requesterName: 'carol', offeredAt: 0 },
    ])
    applyFriendList(roster)
    expect(get(pendingFriendRequests).map((r) => r.requesterName)).toEqual([
      'carol',
    ])
  })

  it('restarts the online diff, so a rejoin announces nobody', () => {
    applyFriendList(roster)
    applyFriendsOnline([{ character_id: 1, level: 12 }], roster)
    applyFriendList([])
    applyFriendList(roster)
    expect(
      applyFriendsOnline([{ character_id: 1, level: 12 }], roster)
    ).toEqual([])
  })
})

describe('sortFriends', () => {
  it('puts the online first, then sorts by name', () => {
    const friends: FriendEntry[] = [
      { characterId: 1, name: 'zoe', level: 1, class: 'knight' },
      { characterId: 2, name: 'amy', level: 1, class: 'ranger' },
      { characterId: 3, name: 'bob', level: 1, class: 'knight' },
    ]
    const online = new Map([[1, 1]])
    expect(sortFriends(friends, online).map((f) => f.name)).toEqual([
      'zoe',
      'amy',
      'bob',
    ])
  })

  it('leaves the input array untouched', () => {
    const friends = [...roster]
    sortFriends(friends, new Map([[2, 8]]))
    expect(friends.map((f) => f.name)).toEqual(['alice', 'bob'])
  })
})

describe('metAgo', () => {
  const now = 1_700_000_000_000

  it('buckets seconds, minutes, hours and days', () => {
    const at = (secondsAgo: number) => now / 1000 - secondsAgo
    expect(metAgo(at(0), now)).toBe('just now')
    expect(metAgo(at(59), now)).toBe('just now')
    expect(metAgo(at(60), now)).toBe('1m ago')
    expect(metAgo(at(3599), now)).toBe('59m ago')
    expect(metAgo(at(3600), now)).toBe('1h ago')
    expect(metAgo(at(86400 * 3), now)).toBe('3d ago')
  })

  it('treats a future timestamp (clock skew) as just now', () => {
    expect(metAgo(now / 1000 + 120, now)).toBe('just now')
  })
})

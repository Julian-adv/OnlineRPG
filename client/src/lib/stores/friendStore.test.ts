import { describe, expect, it, beforeEach } from 'vitest'
import { get } from 'svelte/store'
import {
  applyFriendList,
  applyFriendsOnline,
  friendList,
  newlyOnlineNames,
  onlineFriends,
  pendingFriendRequests,
  resetFriendStores,
  sortFriends,
  type FriendEntry,
} from './friendStore'

const roster: FriendEntry[] = [
  { characterId: 1, name: 'alice', level: 12 },
  { characterId: 2, name: 'bob', level: 8 },
]

describe('newlyOnlineNames', () => {
  it('reports nothing before the first answer', () => {
    // Otherwise every friend already online would look like a new arrival.
    expect(newlyOnlineNames(null, [1, 2], new Map([[1, 'alice']]))).toEqual([])
  })

  it('reports only ids absent from the previous answer', () => {
    const names = new Map([
      [1, 'alice'],
      [2, 'bob'],
    ])
    expect(newlyOnlineNames(new Set([1]), [1, 2], names)).toEqual(['bob'])
  })

  it('reports nothing when the set is unchanged', () => {
    expect(
      newlyOnlineNames(new Set([1]), [1], new Map([[1, 'alice']]))
    ).toEqual([])
  })

  it('skips ids the roster cannot name', () => {
    expect(newlyOnlineNames(new Set(), [9], new Map([[1, 'alice']]))).toEqual(
      []
    )
  })
})

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
      { characterId: 1, name: 'zoe', level: 1 },
      { characterId: 2, name: 'amy', level: 1 },
      { characterId: 3, name: 'bob', level: 1 },
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

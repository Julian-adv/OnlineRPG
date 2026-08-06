import { writable } from 'svelte/store'

/** One friend from ServerMessage::FriendList. Keyed by character id: a
 *  friendship outlives both sessions, and offline friends have no player id. */
export interface FriendEntry {
  characterId: number
  name: string
  level: number
}

/** A friend request the player hasn't answered yet. */
export interface PendingFriendRequest {
  requesterId: number
  requesterName: string
  offeredAt: number
}

/** Mirrors the server-side `FRIEND_REQUEST_TTL`. */
export const FRIEND_REQUEST_TTL_MS = 120_000

/** Mirrors the server-side `FRIEND_PENDING_REQUEST_CAP`. */
export const MAX_PENDING_FRIEND_REQUESTS = 5

/** Mirrors the server-side `MAX_FRIENDS`; shown, not enforced, here. */
export const MAX_FRIENDS = 100

/** Poll periods. There is no presence push, so the closed-panel period is what
 *  the online notice costs when nobody is looking at the list. */
export const FRIENDS_POLL_OPEN_MS = 15_000
export const FRIENDS_POLL_CLOSED_MS = 60_000

/** The whole roster, offline friends included, newest snapshot wins. */
export const friendList = writable<FriendEntry[]>([])

/** character id → live level, for the friends online right now. Absence is
 *  the offline signal. */
export const onlineFriends = writable<Map<number, number>>(new Map())

/** Unanswered requests, oldest first — the toast shows the head, so a flood
 *  can neither swap the name under a click nor bury a legitimate request. */
export const pendingFriendRequests = writable<PendingFriendRequest[]>([])

export const friendPanelVisible = writable(false)

const ONLINE_NOTICE_KEY = 'friendOnlineNotice'

function storedNoticeEnabled(): boolean {
  try {
    return localStorage.getItem(ONLINE_NOTICE_KEY) !== 'off'
  } catch {
    return true
  }
}

/** Whether a friend coming online prints a chat line. Client-side only: the
 *  server pushes nothing either way, so this hides a notice rather than
 *  hiding the player. */
export const friendOnlineNoticeEnabled = writable(storedNoticeEnabled())

friendOnlineNoticeEnabled.subscribe((enabled) => {
  try {
    localStorage.setItem(ONLINE_NOTICE_KEY, enabled ? 'on' : 'off')
  } catch {
    // Private-mode storage refusal; the preference just won't persist.
  }
})

/** Ids in the last answered poll, or null before the first one. Null reports
 *  nothing: on the session's first answer every online friend would otherwise
 *  look like they had just arrived. */
let lastOnlineIds: Set<number> | null = null

/** Which friends became online between two poll answers — the whole online
 *  notice, kept pure so it is testable without a socket. */
export function newlyOnlineNames(
  previous: Set<number> | null,
  current: Iterable<number>,
  nameOf: Map<number, string>
): string[] {
  if (previous === null) return []
  const names: string[] = []
  for (const id of current) {
    if (!previous.has(id)) {
      const name = nameOf.get(id)
      if (name !== undefined) names.push(name)
    }
  }
  return names
}

/** Apply a poll answer and return the names to announce. */
export function applyFriendsOnline(
  entries: { character_id: number; level: number }[],
  roster: FriendEntry[]
): string[] {
  const nameOf = new Map(roster.map((f) => [f.characterId, f.name]))
  const ids = entries.map((e) => e.character_id)
  const announced = newlyOnlineNames(lastOnlineIds, ids, nameOf)
  lastOnlineIds = new Set(ids)
  onlineFriends.set(new Map(entries.map((e) => [e.character_id, e.level])))
  return announced
}

/** A roster with no friends left cannot be corrected by a later poll — the
 *  server stops answering them — so the presence map is cleared here. */
export function applyFriendList(friends: FriendEntry[]) {
  friendList.set(friends)
  // Two people who asked each other become friends with no answer given, so
  // nothing else would take the requester's toast down.
  const names = new Set(friends.map((f) => f.name))
  pendingFriendRequests.update((queue) =>
    queue.filter((request) => !names.has(request.requesterName))
  )
  if (friends.length === 0) {
    lastOnlineIds = null
    onlineFriends.set(new Map())
  }
}

/** Full reset for session-death paths (logout, reconnect, GameState snapshot);
 *  missing one of these is how phantom friend UI survives. */
export function resetFriendStores() {
  friendList.set([])
  onlineFriends.set(new Map())
  pendingFriendRequests.set([])
  friendPanelVisible.set(false)
  lastOnlineIds = null
}

/** Online first, then by name — the order the panel and `/friend` both use. */
export function sortFriends(
  friends: FriendEntry[],
  online: Map<number, number>
): FriendEntry[] {
  return [...friends].sort((a, b) => {
    const aOnline = online.has(a.characterId)
    const bOnline = online.has(b.characterId)
    if (aOnline !== bOnline) return aOnline ? -1 : 1
    return a.name.localeCompare(b.name)
  })
}

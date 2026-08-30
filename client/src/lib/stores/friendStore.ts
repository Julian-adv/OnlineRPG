import { writable } from 'svelte/store'
import { persistedBoolean } from './persisted'
import type { CharacterClass } from '../network/networkTypes'

/** One friend from ServerMessage::FriendList. Keyed by character id: a
 *  friendship outlives both sessions, and offline friends have no player id. */
export interface FriendEntry {
  characterId: number
  name: string
  level: number
  class: CharacterClass
}

/** One remembered meeting from ServerMessage::RecentEncounters. The name is
 *  a snapshot from the last meeting. */
export interface EncounterEntry {
  characterId: number
  name: string
  lastMetUnix: number
  metCount: number
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

/** The last RecentEncounters answer, newest first — server order, kept as
 *  delivered. */
export const recentEncounters = writable<EncounterEntry[]>([])

/** "just now" / "5m ago" / "3h ago" / "2d ago" for the Met tab. */
export function metAgo(lastMetUnix: number, nowMs: number): string {
  const seconds = Math.max(0, Math.floor(nowMs / 1000) - lastMetUnix)
  if (seconds < 60) return 'just now'
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`
  return `${Math.floor(seconds / 86400)}d ago`
}

/** Whether a friend coming online prints a chat line. Client-side only: the
 *  server pushes nothing either way, so this hides a notice rather than
 *  hiding the player. */
export const friendOnlineNoticeEnabled = persistedBoolean(
  'friendOnlineNotice',
  true
)

/** Ids in the last answered poll, or null before the first one. Null reports
 *  nothing: on the session's first answer every online friend would otherwise
 *  look like they had just arrived. */
let lastOnlineIds: Set<number> | null = null

/** Apply a poll answer and return the names to announce: friends online now
 *  that were absent from the previous answer. */
export function applyFriendsOnline(
  entries: { character_id: number; level: number }[],
  roster: FriendEntry[]
): string[] {
  const announced: string[] = []
  if (lastOnlineIds !== null) {
    const previous = lastOnlineIds
    const nameOf = new Map(roster.map((f) => [f.characterId, f.name]))
    for (const { character_id } of entries) {
      if (previous.has(character_id)) continue
      const name = nameOf.get(character_id)
      if (name !== undefined) announced.push(name)
    }
  }
  lastOnlineIds = new Set(entries.map((e) => e.character_id))
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
  recentEncounters.set([])
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

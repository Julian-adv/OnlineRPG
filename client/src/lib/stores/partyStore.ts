import { writable } from 'svelte/store'

/** One member as listed in ServerMessage::PartyState. */
export interface PartyMemberEntry {
  id: number
  name: string
}

/** The player's party roster, driven by PartyState snapshots; null when not
 *  in a party (the server sends an empty roster to clear it). */
export interface PartyRoster {
  leaderId: number
  members: PartyMemberEntry[]
}

export const partyRoster = writable<PartyRoster | null>(null)

/** One member's location from ServerMessage::PartyPositions. */
export interface PartyMemberPositionEntry {
  id: number
  x: number
  z: number
  floor_level: number
}

/** Latest positions poll answer, stamped on receipt. Rendering joins it
 *  against the roster (a leaver never draws) and age-gates it (an answer
 *  that landed after the map closed never outlives its freshness). */
export interface PartyPositionsSnapshot {
  at: number
  members: PartyMemberPositionEntry[]
}

export const partyPositions = writable<PartyPositionsSnapshot>({
  at: 0,
  members: [],
})

/** A party invite the player hasn't answered yet. */
export interface PendingPartyInvite {
  inviterId: number
  inviterName: string
  offeredAt: number
}

/** Unanswered invites, oldest first — the toast shows the head, so a flood
 *  can neither swap the name under a click nor bury a legitimate invite. */
export const pendingPartyInvites = writable<PendingPartyInvite[]>([])

export const MAX_PENDING_PARTY_INVITES = 3

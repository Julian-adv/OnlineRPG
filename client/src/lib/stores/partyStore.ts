import { writable } from 'svelte/store'
import type { CharacterClass } from '../network/networkTypes'

/** One member as listed in ServerMessage::PartyState; hp is kept current
 *  by PartyVitals pushes. */
export interface PartyMemberEntry {
  id: number
  name: string
  hp: number
  max_hp: number
  class: CharacterClass
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

/** Latest pushed locations of the *other* party members (self is filtered
 *  on receipt). Validity is event-driven — server pushes on relocation and
 *  membership change, and the roster join drops leavers — so there is no
 *  freshness gate. */
export const partyPositions = writable<PartyMemberPositionEntry[]>([])

/** Apply a pushed party-wide payload. A push can cross a disband on the
 *  wire, so without a roster it is dropped; the server serializes one
 *  payload for the whole party, so self is filtered here. */
export function applyPartyPositions(
  members: PartyMemberPositionEntry[],
  selfId: number | undefined,
  inParty: boolean
) {
  if (!inParty) return
  partyPositions.set(members.filter((m) => m.id !== selfId))
}

export function resetPartyPositions() {
  partyPositions.set([])
}

/** One member's health from ServerMessage::PartyVitals. */
export interface PartyMemberVitalsEntry {
  id: number
  hp: number
  max_hp: number
}

/** Fold a vitals push into the roster. Ids absent from the roster are
 *  dropped: a push can cross a roster change on the wire, and the next
 *  PartyState re-seeds every bar anyway. */
export function applyPartyVitals(vitals: PartyMemberVitalsEntry[]) {
  partyRoster.update((roster) => {
    if (!roster) return roster
    const byId = new Map(vitals.map((v) => [v.id, v]))
    return {
      ...roster,
      members: roster.members.map((m) => {
        const v = byId.get(m.id)
        return v ? { ...m, hp: v.hp, max_hp: v.max_hp } : m
      }),
    }
  })
}

/** Full party reset for session-death paths (logout, reconnect, GameState
 *  snapshot); missing any one store here is how phantom party UI survives. */
export function resetPartyStores() {
  partyRoster.set(null)
  resetPartyPositions()
  pendingPartyInvites.set([])
  pendingPartySummons.set([])
}

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

/** Mirrors the server-side cap (`PARTY_MAX_MEMBERS` in party.rs). */
export const PARTY_MAX_MEMBERS = 8

/** Classes with an icon in /icons/party/; others (agent-created classes,
 *  e.g. samurai) fall back to a letter badge. */
const CLASS_ICONS: Partial<Record<CharacterClass, string>> = {
  knight: '/icons/party/class-knight.svg',
  barbarian: '/icons/party/class-barbarian.svg',
  caveman: '/icons/party/class-caveman.svg',
  valkyrie: '/icons/party/class-valkyrie.svg',
  ranger: '/icons/party/class-ranger.svg',
  priest: '/icons/party/class-priest.svg',
  rogue: '/icons/party/class-rogue.svg',
  bard: '/icons/party/class-bard.svg',
}

export function classIconPath(cls: string): string | null {
  return CLASS_ICONS[cls as CharacterClass] ?? null
}

/** Mirrors the server-side invite TTL (`PARTY_INVITE_TTL`). */
export const INVITE_TTL_MS = 30_000

/** Mirrors the server-side summon TTL (`PARTY_SUMMON_TTL`). */
export const SUMMON_TTL_MS = 30_000

/** A summoning-scroll consent request the player hasn't answered yet. */
export interface PendingPartySummon {
  casterId: number
  casterName: string
  offeredAt: number
}

/** Unanswered summons, oldest first, same queue discipline as invites. */
export const pendingPartySummons = writable<PendingPartySummon[]>([])

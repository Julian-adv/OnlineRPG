import { writable } from 'svelte/store'

export interface TeleportGateDestination {
  gateId: string
  townName: string
  distanceM: number
  fare: number
}

export interface TeleportGateSession {
  gateId: string
  townName: string
  destinations: TeleportGateDestination[]
  misfireChanceBps: number
}

export const teleportGateSession = writable<TeleportGateSession | null>(null)
export const teleportGateBusy = writable(false)

export function resetTeleportGateStore() {
  teleportGateSession.set(null)
  teleportGateBusy.set(false)
}

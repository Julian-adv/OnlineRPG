import type { RemotePlayer } from '../stores/gameStore'
import { setBardZone } from './bgmManager'

/** The playlist yields to a bard NPC's earshot. AOI membership *is* the
 *  delivery circle — PlayerAppeared/Disappeared fire exactly at
 *  EVENT_DELIVERY_RADIUS, per floor — so recompute on every roster change
 *  instead of re-deriving the server's geometry. */
export function refreshBardZone(others: ReadonlyMap<number, RemotePlayer>) {
  let near = false
  for (const p of others.values()) {
    if (p.isOfficialNpc && p.characterClass === 'bard') {
      near = true
      break
    }
  }
  setBardZone(near)
}

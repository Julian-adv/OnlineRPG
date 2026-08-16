import { derived, writable } from 'svelte/store'
import {
  debuffPresentation,
  formatRemaining,
  type PresentedDebuff,
} from '../data/debuffPresentation'

/** The local player's active debuffs (doc/DEBUFF.md), pushed by the server on
 *  apply/refresh/expiry. `until` is on the Date.now clock. */
export interface ActiveDebuff {
  id: string
  until: number
}

export const activeDebuffs = writable<ActiveDebuff[]>([])

/** Unexpired debuffs with their presentation and a live remaining-time label;
 *  ticks once a second only while something is active. */
export const visibleDebuffs = derived<typeof activeDebuffs, PresentedDebuff[]>(
  activeDebuffs,
  ($active, set) => {
    const update = () => {
      const now = Date.now()
      set(
        $active
          .filter((d) => d.until > now)
          .map((d) => ({
            ...debuffPresentation(d.id),
            id: d.id,
            remaining: formatRemaining(d.until - now),
          }))
      )
    }
    update()
    if ($active.length === 0) return
    const timer = setInterval(update, 1_000)
    return () => clearInterval(timer)
  },
  []
)

export function resetDebuffStore() {
  activeDebuffs.set([])
}

/** XP the badge has not shown yet: it waits for the kill's death animation to
 *  start, so the gauge spark rides that instead of the wire message. */
export interface XpArrival {
  level: number
  totalXp: number
  /** Combat-log lines that belong to this XP; they land with it, not before. */
  lines: string[]
}

// A kill that never visibly falls (despawn, AOI) must not strand the badge on
// the old number. Set clear of the longest wait before the death clip starts:
// PLAYER_ATTACK_IMPACT_DELAY_MS plus the hit reaction.
export const XP_ARRIVAL_TIMEOUT_MS = 2000

let pending: XpArrival | null = null
let deliver: ((xp: XpArrival) => void) | null = null
let waitingFor = new Set<string>()
let timer: ReturnType<typeof setTimeout> | undefined

/** Concurrent kill shares can arrive out of XP order, so the queue keeps the
 *  highest total rather than the newest — but every kill's lines are kept. */
export function queueXpArrival(
  next: XpArrival,
  monsterId: string,
  onArrive: (xp: XpArrival) => void
) {
  waitingFor.add(monsterId)
  if (pending) {
    const ahead = next.totalXp >= pending.totalXp ? next : pending
    pending = {
      level: ahead.level,
      totalXp: ahead.totalXp,
      lines: [...pending.lines, ...next.lines],
    }
  } else {
    pending = next
  }
  deliver = onArrive
  clearTimeout(timer)
  timer = setTimeout(() => releaseXpArrival(), XP_ARRIVAL_TIMEOUT_MS)
}

/** With `monsterId`, releases only XP that was waiting on that kill; without
 *  one, releases whatever is held. */
export function releaseXpArrival(monsterId?: string) {
  if (monsterId !== undefined && !waitingFor.has(monsterId)) return
  const arrival = pending
  const onArrive = deliver
  clearXpArrival()
  if (arrival && onArrive) onArrive(arrival)
}

/** Drop the held XP without showing it — the world it belonged to is gone. */
export function clearXpArrival() {
  clearTimeout(timer)
  timer = undefined
  pending = null
  deliver = null
  waitingFor = new Set()
}

export function hasPendingXpArrival(): boolean {
  return pending !== null
}

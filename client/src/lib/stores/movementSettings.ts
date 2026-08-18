import { persistedBoolean } from './persisted'

/** Run without holding Shift. On by default; persisted per browser. */
export const alwaysRun = persistedBoolean('onlinerpg_alwaysRun', true)

// Cached: sprintRequested runs per frame, so it must not subscribe each call.
let alwaysRunNow = false
alwaysRun.subscribe((v) => (alwaysRunNow = v))

/** Shift inverts the preference: it means "walk" while always-run is on. */
export function sprintRequested(shiftHeld: boolean): boolean {
  return alwaysRunNow ? !shiftHeld : shiftHeld
}

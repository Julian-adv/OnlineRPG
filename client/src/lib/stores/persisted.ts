import { writable, type Writable } from 'svelte/store'

/** A boolean store persisted per browser. Storage refusals (private mode,
 *  SSR) just mean the preference won't survive a reload. */
export function persistedBoolean(
  key: string,
  defaultValue: boolean
): Writable<boolean> {
  let initial = defaultValue
  try {
    const stored = localStorage.getItem(key)
    if (stored !== null) initial = stored === 'true'
  } catch {
    // unavailable storage; fall back to the default
  }
  const store = writable(initial)
  store.subscribe((value) => {
    try {
      localStorage.setItem(key, String(value))
    } catch {
      // unavailable storage; the preference just won't persist
    }
  })
  return store
}

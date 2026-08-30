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

/** A string store persisted per browser; `accept` guards against a stale or
 *  hand-edited value so the app never starts on an unknown option. */
export function persistedString<T extends string>(
  key: string,
  defaultValue: T,
  accept: (value: string) => value is T
): Writable<T> {
  let initial = defaultValue
  try {
    const stored = localStorage.getItem(key)
    if (stored !== null && accept(stored)) initial = stored
  } catch {
    // unavailable storage; fall back to the default
  }
  const store = writable<T>(initial)
  store.subscribe((value) => {
    try {
      localStorage.setItem(key, value)
    } catch {
      // unavailable storage; the preference just won't persist
    }
  })
  return store
}

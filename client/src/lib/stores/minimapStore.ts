import { writable } from 'svelte/store'

const STORAGE_KEY = 'onlinerpg_minimapEnabled'

/** HUD minimap toggle. Off by default (issue #11); persisted per browser. */
export const minimapEnabled = writable<boolean>(
  typeof localStorage !== 'undefined' &&
    localStorage.getItem(STORAGE_KEY) === 'true'
)

minimapEnabled.subscribe((v) => {
  if (typeof localStorage === 'undefined') return
  localStorage.setItem(STORAGE_KEY, String(v))
})

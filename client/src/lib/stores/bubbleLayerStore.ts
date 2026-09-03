import { writable } from 'svelte/store'

/** HUD element chat bubbles portal into: above the minimap, below every panel. */
export const bubbleLayer = writable<HTMLElement | null>(null)

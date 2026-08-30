import { writable } from 'svelte/store'

/** The local character's earned title ids (doc/TITLES.md); the shown one
 *  lives on `gameStore.currentPlayer.title`. */
export const earnedTitles = writable<string[]>([])

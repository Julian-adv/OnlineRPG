import { writable } from 'svelte/store'

/** Clip name an emote chat command wants the local player to play.
 *  `PlayerControl` consumes it and clears it back to null. Emotes go through
 *  a store rather than `sendInteractObject` because that path carries an
 *  object id, and the server rejects a second player claiming the same one
 *  ("occupied") — two players can play music side by side. */
export const emoteRequest = writable<string | null>(null)

import { writable } from 'svelte/store'

/** Clip name an emote chat command wants the local player to play.
 *  `PlayerControl` consumes it and clears it back to null. Emotes go through
 *  a store rather than `sendInteractObject` because that path carries an
 *  object id, and the server rejects a second player claiming the same one
 *  ("occupied") — two players can play music side by side. */
export const emoteRequest = writable<string | null>(null)

/** Set when an emote should end on its own — `/play_music` does it when the
 *  track runs out. `PlayerControl` leaves the interaction and clears it. */
export const emoteStopRequest = writable(false)

/** Clip the `/play_music` emote holds, and the interaction the server stores
 *  for it. Must match `MUSIC_EMOTE` in `shared/src/messages.rs` — the server
 *  and agent-client read it from there. */
export const MUSIC_EMOTE_ANIM = 'guitar_playing'

/** One-shot clips `/emote <name>` plays. Must match `ONE_SHOT_EMOTES` in
 *  `shared/src/messages.rs` — the server validates the command against it. */
export const ONE_SHOT_EMOTE_ANIMS = new Set(['excited', 'clap'])

/** Clips `/emote <name>` loops until the player moves or presses Escape.
 *  Must match `LOOPING_EMOTES` in `shared/src/messages.rs`. */
export const LOOPING_EMOTE_ANIMS = new Set(['twist', 'macarena', 'chicken'])

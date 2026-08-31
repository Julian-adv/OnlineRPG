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

/** The emote panel (HUD social menu / G key). */
export const emotePanelVisible = writable(false)

/** Anim the local player is currently emoting, for the panel's highlight.
 *  `PlayerControl` owns it: set on entering an emote, cleared on exit. */
export const localEmoteAnim = writable<string | null>(null)

/** Clip the `/play_music` emote holds, and the interaction the server stores
 *  for it. Must match `MUSIC_EMOTE` in `shared/src/messages.rs` — the server
 *  and agent-client read it from there. */
export const MUSIC_EMOTE_ANIM = 'guitar_playing'

/** One-shot clips `/emote <name>` plays. Must match `ONE_SHOT_EMOTES` in
 *  `shared/src/messages.rs` — the server validates the command against it. */
export const ONE_SHOT_EMOTE_ANIMS = new Set(['excited', 'clap', 'yawn'])

/** Clips `/emote <name>` loops until the player moves or presses Escape.
 *  Must match `LOOPING_EMOTES` in `shared/src/messages.rs`. */
export const LOOPING_EMOTE_ANIMS = new Set([
  'twist',
  'macarena',
  'chicken',
  'stand_pose2',
  'stand_pose3',
  'stand_pose4',
  'weight_shift',
])

/** Idle clips `/emote` also accepts for eyeballing clips in-game — one-shot,
 *  but hidden from the panel and the advertised lists. Must match
 *  `DEBUG_EMOTES` in `shared/src/messages.rs`. Clips come from the
 *  locomotion/combat packs, not `social.glb`. */
export const DEBUG_EMOTE_ANIMS = new Set([
  'idle1',
  'idle2',
  'idle3',
  'idle4',
  'idle5',
  'combat_idle',
])

/** Everything `/emote` accepts — the server's validation list. */
export const SLASH_EMOTE_ANIMS = new Set([
  ...ONE_SHOT_EMOTE_ANIMS,
  ...LOOPING_EMOTE_ANIMS,
  ...DEBUG_EMOTE_ANIMS,
])

/** Emotes that play once and end on their own — the advertised one-shots
 *  plus the hidden debug clips. The complement of the held poses. */
export const SELF_ENDING_EMOTE_ANIMS = new Set([
  ...ONE_SHOT_EMOTE_ANIMS,
  ...DEBUG_EMOTE_ANIMS,
])

/** Performances held until the player moves or presses Escape. */
export const HELD_EMOTE_ANIMS = new Set([
  MUSIC_EMOTE_ANIM,
  ...LOOPING_EMOTE_ANIMS,
])

/** Every emote clip. Unlike placed-object interactions, these play where the
 *  player stands. */
export const EMOTE_ANIMS = new Set([MUSIC_EMOTE_ANIM, ...SLASH_EMOTE_ANIMS])

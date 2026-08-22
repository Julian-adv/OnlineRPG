import { LOOPING_EMOTE_ANIMS, SLASH_EMOTE_ANIMS } from './stores/emoteStore'

/** Panel entries derived from the wire contract's anim lists (emoteStore
 *  mirrors shared/src/messages.rs), so a new server emote appears here with
 *  no per-emote client work. */
export interface EmoteMeta {
  anim: string
  label: string
  /** Loops until the player moves or stops it; one-shots end on their own. */
  loops: boolean
}

function labelFor(anim: string): string {
  return anim
    .split('_')
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ')
}

export const EMOTE_LIST: EmoteMeta[] = [...SLASH_EMOTE_ANIMS].map((anim) => ({
  anim,
  label: labelFor(anim),
  loops: LOOPING_EMOTE_ANIMS.has(anim),
}))

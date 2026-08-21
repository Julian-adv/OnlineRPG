import { LOOPING_EMOTE_ANIMS, ONE_SHOT_EMOTE_ANIMS } from './stores/emoteStore'

/** Presentation metadata for the emote panel. The anims themselves live in
 *  emoteStore (mirroring shared/src/messages.rs); this file only names them. */
export interface EmoteMeta {
  anim: string
  label: string
  glyph: string
  /** Loops until the player moves or stops it; one-shots end on their own. */
  loops: boolean
}

export const EMOTE_LIST: EmoteMeta[] = [
  { anim: 'excited', label: 'Excited', glyph: '🙌', loops: false },
  { anim: 'clap', label: 'Clap', glyph: '👏', loops: false },
  { anim: 'twist', label: 'Twist', glyph: '🕺', loops: true },
  { anim: 'macarena', label: 'Macarena', glyph: '💃', loops: true },
  { anim: 'chicken', label: 'Chicken', glyph: '🐔', loops: true },
]

/** Guard used by tests: the panel must cover exactly what `/emote` accepts. */
export function emoteListMatchesServer(): boolean {
  const anims = new Set(EMOTE_LIST.map((e) => e.anim))
  const server = new Set([...ONE_SHOT_EMOTE_ANIMS, ...LOOPING_EMOTE_ANIMS])
  return (
    anims.size === server.size &&
    [...server].every((a) => anims.has(a)) &&
    EMOTE_LIST.every((e) => e.loops === LOOPING_EMOTE_ANIMS.has(e.anim))
  )
}

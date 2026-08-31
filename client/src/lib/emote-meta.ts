import {
  LOOPING_EMOTE_ANIMS,
  ONE_SHOT_EMOTE_ANIMS,
} from './stores/emoteStore'

/** Panel entries derived from the wire contract's anim lists (emoteStore
 *  mirrors shared/src/messages.rs), so a new server emote appears here with
 *  no per-emote client work. */
export interface EmoteMeta {
  anim: string
  label: string
  /** Loops until the player moves or stops it; one-shots end on their own. */
  loops: boolean
}

/** Anims whose mechanical label reads badly (`stand_pose2` → "Stand Pose2"). */
const LABEL_OVERRIDES: Record<string, string> = {
  stand_pose2: 'Pose 2',
  stand_pose3: 'Pose 3',
  stand_pose4: 'Pose 4',
}

function labelFor(anim: string): string {
  return (
    LABEL_OVERRIDES[anim] ??
    anim
      .split('_')
      .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
      .join(' ')
  )
}

export const EMOTE_LIST: EmoteMeta[] = [
  ...ONE_SHOT_EMOTE_ANIMS,
  ...LOOPING_EMOTE_ANIMS,
].map((anim) => ({
  anim,
  label: labelFor(anim),
  loops: LOOPING_EMOTE_ANIMS.has(anim),
}))

/** The player's last click — anim they commanded (null = stop) and when. */
export interface EmoteIntent {
  anim: string | null
  at: number
}

/** After this long an unconfirmed intent is assumed rejected by the server. */
export const EMOTE_INTENT_TTL_MS = 2000

/** Whether clicking `emote` should stop or play, judged against the last
 *  commanded intent rather than the server echo — the echo lags a round
 *  trip, and deciding on stale state turns fast clicks into wrong stops
 *  (or dead re-stops) while an earlier command is still in flight. */
export function emoteClickCommand(
  emote: EmoteMeta,
  active: string | null,
  intent: EmoteIntent | null,
  now: number
): 'stop' | 'play' {
  const current =
    intent && now - intent.at <= EMOTE_INTENT_TTL_MS ? intent.anim : active
  return emote.loops && current === emote.anim ? 'stop' : 'play'
}

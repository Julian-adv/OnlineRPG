import {
  playPerformance,
  stopPerformance,
  fadeOutPerformance,
} from './bgmManager'
import { emoteStopRequest, MUSIC_EMOTE_ANIM } from '../stores/emoteStore'

/** Whose `/play_music` we are listening to. Several bards can strum at once,
 *  but only the newest one is audible — so the stop only counts when it comes
 *  from the player holding the slot. */
let performerId: number | null = null

/** Bumped whenever the local player starts a track, so the end callback of a
 *  performance we already replaced cannot cancel the new emote. */
let myPerformance = 0

export function startMusicPerformance(
  playerId: number,
  track: string,
  isSelf: boolean,
  elapsedSecs = 0
) {
  let onEnded: (() => void) | undefined
  if (isSelf) {
    const performance = ++myPerformance
    onEnded = () => {
      if (performance !== myPerformance) return
      // Leaving the emote sends StopInteraction, which ends the strum
      // animation for everyone nearby.
      emoteStopRequest.set(true)
    }
  }
  if (playPerformance(track, onEnded, elapsedSecs)) {
    performerId = playerId
  }
}

/** A player's interaction changed: anything but the strum — cleared, or a
 *  bench taken instead — means their tune is over. */
export function applyInteractionChange(
  playerId: number,
  objectType: string | null
) {
  if (objectType === MUSIC_EMOTE_ANIM) return
  stopMusicPerformance(playerId)
}

/** The performer moved, left, or their track ran out. */
export function stopMusicPerformance(playerId: number) {
  if (performerId !== playerId) return
  performerId = null
  stopPerformance()
}

/** Earshot was lost by distance rather than by the tune ending — fade out
 *  instead of cutting. */
export function fadeOutMusicPerformance(playerId: number) {
  if (performerId !== playerId) return
  performerId = null
  fadeOutPerformance()
}

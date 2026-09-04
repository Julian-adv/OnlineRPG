import type { ClickIntent } from '../../../managers/inputHandler'
import type { Position } from '../../../utils/movementUtils'
import type { PendingApproach } from './approach'
import {
  dispatchCanvasClickIntent,
  type CanvasClickActions,
} from '../canvas-click-dispatcher'
import type { PlayerControlEvent, PlayerControlUpdateOptions } from '../events'

export type { PlayerControlEvent, PlayerControlUpdateOptions }

// ───────────────────────────────────────────────────────────────────────────
// Canvas click → PlayerControlEvent intent
// ───────────────────────────────────────────────────────────────────────────

interface CanvasClickPlayer {
  health: number
}

interface CreateCanvasIntentEventInput {
  event: MouseEvent
  editorMode: boolean
  currentPlayer: CanvasClickPlayer | null
  processIntent: () => ClickIntent
}

export function createCanvasIntentEvent({
  event,
  editorMode,
  currentPlayer,
  processIntent,
}: CreateCanvasIntentEventInput): PlayerControlEvent | null {
  const expectedButton = editorMode ? 2 : 0
  if (event.button !== expectedButton) return null
  if (!currentPlayer || currentPlayer.health <= 0) return null

  return {
    type: 'canvas_intent',
    intent: processIntent(),
    editorMode,
  }
}

// ───────────────────────────────────────────────────────────────────────────
// PlayerControlEvent dispatch (queue drain → action routing)
// ───────────────────────────────────────────────────────────────────────────

export interface PlayerControlEventActions extends CanvasClickActions {
  requestMove(
    position: Position,
    options?: { approach?: PendingApproach | null }
  ): void
  onInteractionFinished(): void
  onPickupGrab(): void
  onInteractionRejected(): void
}

export function dispatchPlayerControlEvent(
  event: PlayerControlEvent,
  actions: PlayerControlEventActions,
  attackRange: number
) {
  switch (event.type) {
    case 'canvas_intent':
      dispatchCanvasClickIntent(
        event.intent,
        event.editorMode,
        actions,
        attackRange
      )
      return
    case 'request_move':
    case 'delayed_request_move':
      actions.requestMove(event.position, {
        approach: event.approach ?? null,
      })
      return
    case 'anim_interaction_finished':
      actions.onInteractionFinished()
      return
    case 'anim_pickup_grab':
      actions.onPickupGrab()
      return
    case 'network_interaction_rejected':
      actions.onInteractionRejected()
      return
    default: {
      const _exhaustive: never = event
      return _exhaustive
    }
  }
}

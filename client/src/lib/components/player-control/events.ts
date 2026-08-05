import type { ClickIntent } from '../../managers/inputHandler'
import type { Position } from '../../utils/movementUtils'

export type PlayerControlEvent =
  | { type: 'canvas_intent'; intent: ClickIntent; editorMode: boolean }
  | {
      type: 'request_move' | 'delayed_request_move'
      position: Position
      pickupAfterArrival?: number | null
      teleportGateAfterArrival?: string | null
    }
  | { type: 'anim_interaction_finished' }
  | { type: 'anim_pickup_grab' }
  | { type: 'network_interaction_rejected' }

export interface PlayerControlUpdateOptions {
  editorMode: boolean
  events?: PlayerControlEvent[]
}

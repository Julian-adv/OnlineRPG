import type { ClickIntent } from '../../managers/inputHandler'
import type { Position } from '../../utils/movementUtils'
import type { PendingApproach } from './fsm/approach'

export type PlayerControlEvent =
  | { type: 'canvas_intent'; intent: ClickIntent; editorMode: boolean }
  | {
      type: 'request_move' | 'delayed_request_move'
      position: Position
      /** Re-armed on the deferred move so a stand-up doesn't drop the action
       *  the click asked for. */
      approach?: PendingApproach | null
    }
  | { type: 'anim_interaction_finished' }
  | { type: 'anim_pickup_grab' }
  | { type: 'network_interaction_rejected' }

export interface PlayerControlUpdateOptions {
  editorMode: boolean
  events?: PlayerControlEvent[]
}

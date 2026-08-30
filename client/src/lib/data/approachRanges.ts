import type { ApproachSpec } from '../components/player-control/fsm/approach'
import { SitAnimationName } from '../types/animations'
import { PLAYER_PICKUP_RANGE_METERS } from './combatTiming'
import {
  NPC_TRADE_RANGE_METERS,
  STALL_TRADE_RANGE_METERS,
  TIP_HAT_RANGE_METERS,
} from './tradeConstants'

/** Walk-up shape per clickable thing: `range` is how close the action fires,
 *  `stopShort` where the walk stops. Ranges stay inside the server's own check
 *  so a position it has yet to catch up with can't get the action refused. */
export type ApproachRange = Omit<ApproachSpec, 'position'>

/** Solid things need clearance to stand in front of; a metre inside the reach
 *  is the default. A ground item is walked onto instead (`stopShort: 0`). */
const reach = (range: number): ApproachRange => ({
  range,
  stopShort: range - 1,
})

/** Must stay under MAX_DOOR_DISTANCE in server/src/game_state/mod.rs, which
 *  measures from the door segment's centre rather than the clicked face. */
export const HOUSE_DOOR_APPROACH: ApproachRange = { range: 1.5, stopShort: 1.0 }

/** Must stay under DOOR_INTERACT_RANGE in server/src/game_state/dungeon.rs. */
export const DUNGEON_DOOR_APPROACH: ApproachRange = {
  range: 2.0,
  stopShort: 1.4,
}

/** Furniture with a pose (bed). The server validates no range of its own. */
export const OBJECT_APPROACH: ApproachRange = { range: 3.0, stopShort: 1.5 }

/** Chairs: the sit-down clip starts ~0.6m in front of the seat, so walk up
 *  close enough that the snap onto it is barely visible. */
const SIT_APPROACH: ApproachRange = { range: 1.5, stopShort: 0.8 }

export function approachForInteraction(interaction: string): ApproachRange {
  return interaction === SitAnimationName.SIT ? SIT_APPROACH : OBJECT_APPROACH
}

export const PICKUP_APPROACH: ApproachRange = {
  range: PLAYER_PICKUP_RANGE_METERS - 0.3,
  stopShort: 0,
}

/** Barrels, crates and chests. The dungeon layer owns the trigger here, so
 *  these only shape the walk-up — far enough back to clear the prop's solid
 *  cell, and inside both PROP_INTERACT_TRIGGER_RANGE (GameSceneDungeonLayer)
 *  and PROP_INTERACT_RANGE in server/src/game_state/dungeon.rs. Equal values
 *  because `range` only feeds the already-close short-circuit. */
export const PROP_APPROACH: ApproachRange = { range: 1.6, stopShort: 1.6 }

export const NPC_TRADE_APPROACH = reach(NPC_TRADE_RANGE_METERS)
export const TIP_HAT_APPROACH = reach(TIP_HAT_RANGE_METERS)
export const STALL_TRADE_APPROACH = reach(STALL_TRADE_RANGE_METERS)

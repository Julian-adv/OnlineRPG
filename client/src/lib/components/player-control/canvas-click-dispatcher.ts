import type { Position } from '../../utils/movementUtils'
import type { ClickIntent } from '../../managers/inputHandler'
import { PLAYER_ATTACK_RANGE_METERS } from '../../data/combatTiming'

type DoorIntent = Extract<ClickIntent, { type: 'toggle_door' }>
type DungeonDoorIntent = Extract<ClickIntent, { type: 'toggle_dungeon_door' }>
type InteractIntent = Extract<ClickIntent, { type: 'interact_object' }>
type PickupIntent = Extract<ClickIntent, { type: 'pickup_ground_item' }>
type NpcIntent = Extract<ClickIntent, { type: 'interact_npc' }>
type BreakPropIntent = Extract<ClickIntent, { type: 'break_prop' }>
type OpenPropIntent = Extract<ClickIntent, { type: 'open_prop' }>
type CastFishingIntent = Extract<ClickIntent, { type: 'cast_fishing' }>
type TipHatIntent = Extract<ClickIntent, { type: 'tip_hat' }>
type StallIntent = Extract<ClickIntent, { type: 'stall' }>
type MealIntent = Extract<ClickIntent, { type: 'meal' }>

export interface CanvasClickActions {
  /** Player is at melee range — start the attack swing immediately. */
  attackInRange(monsterId: string): void
  /** Out of range — chase the monster, attacking on arrival. */
  chaseAndAttack(monsterId: string, hitPoint: Position): void
  /** Walk up to a clicked house door, toggling it on arrival. */
  toggleDoor(intent: DoorIntent): void
  /** Walk up to a clicked dungeon door (entrance at depth 0, or an interior
   *  room door), toggling it via the server on arrival so the swing syncs to
   *  other players. */
  toggleDungeonDoor(intent: DungeonDoorIntent): void
  /** Walk up to a clicked chair/bench, sitting on arrival. */
  interactObject(intent: InteractIntent): void
  /** Walk up to a clicked ground item, picking it up on arrival. */
  pickupItem(intent: PickupIntent): void
  interactNpc(intent: NpcIntent): void
  /** Walk up to a clicked barrel/crate, breaking it on arrival. */
  breakProp(intent: BreakPropIntent): void
  /** Walk up to a clicked chest, opening it (lid animation) on arrival. */
  openProp(intent: OpenPropIntent): void
  moveToGround(position: Position, sprinting: boolean): void
  /** Stop, face the water, and cast the equipped rod (server validates). */
  castFishing(intent: CastFishingIntent): void
  /** Walk up to a clicked tip hat, opening its tip dialog on arrival. */
  tipHat(intent: TipHatIntent): void
  /** Walk up to a clicked stall, opening a trade with its owner on arrival. */
  tradeAtStall(intent: StallIntent): void
  /** Eat a served plate without moving — only from the chair it was served to. */
  eatMeal(intent: MealIntent): void
}

export function dispatchCanvasClickIntent(
  intent: ClickIntent,
  isMapEditorMode: boolean,
  actions: CanvasClickActions
): void {
  if (isMapEditorMode && intent.type !== 'move_to_ground') return

  switch (intent.type) {
    case 'attack_monster':
      if (intent.distance < PLAYER_ATTACK_RANGE_METERS) {
        actions.attackInRange(intent.monsterId)
      } else {
        actions.chaseAndAttack(intent.monsterId, intent.hitPoint)
      }
      return
    case 'toggle_door':
      actions.toggleDoor(intent)
      return
    case 'toggle_dungeon_door':
      actions.toggleDungeonDoor(intent)
      return
    case 'interact_object':
      actions.interactObject(intent)
      return
    case 'pickup_ground_item':
      actions.pickupItem(intent)
      return
    case 'interact_npc':
      actions.interactNpc(intent)
      return
    case 'break_prop':
      actions.breakProp(intent)
      return
    case 'open_prop':
      actions.openProp(intent)
      return
    case 'move_to_ground':
      actions.moveToGround(intent.position, intent.sprinting)
      return
    case 'cast_fishing':
      actions.castFishing(intent)
      return
    case 'tip_hat':
      actions.tipHat(intent)
      return
    case 'stall':
      actions.tradeAtStall(intent)
      return
    case 'meal':
      actions.eatMeal(intent)
      return
    case 'none':
      return
    default: {
      const _exhaustive: never = intent
      return _exhaustive
    }
  }
}

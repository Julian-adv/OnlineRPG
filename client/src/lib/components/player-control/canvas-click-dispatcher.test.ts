import { describe, expect, it, vi } from 'vitest'
import type { ClickIntent } from '../../managers/inputHandler'
import {
  dispatchCanvasClickIntent,
  type CanvasClickActions,
} from './canvas-click-dispatcher'
import { PLAYER_ATTACK_RANGE_METERS } from '../../data/combatTiming'

function makeActions() {
  return {
    attackInRange: vi.fn(),
    chaseAndAttack: vi.fn(),
    toggleDoor: vi.fn(),
    toggleDungeonDoor: vi.fn(),
    interactObject: vi.fn(),
    pickupItem: vi.fn(),
    interactNpc: vi.fn(),
    breakProp: vi.fn(),
    openProp: vi.fn(),
    moveToGround: vi.fn(),
    castFishing: vi.fn(),
    tipHat: vi.fn(),
    tradeAtStall: vi.fn(),
    eatMeal: vi.fn(),
  } satisfies CanvasClickActions
}

describe('dispatchCanvasClickIntent tip hats', () => {
  it('routes a tip_hat intent to tipHat', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'tip_hat',
      hatId: 7,
      position: { x: 1, y: 0, z: 2 },
    }

    dispatchCanvasClickIntent(
      intent,
      false,
      actions,
      PLAYER_ATTACK_RANGE_METERS
    )

    expect(actions.tipHat).toHaveBeenCalledWith(intent)
  })
})

describe('dispatchCanvasClickIntent prop handling', () => {
  it('routes a break_prop intent to breakProp', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'break_prop',
      entranceId: 'd1',
      depth: 1,
      propId: 3,
      position: { x: 1, y: 0, z: 2 },
    }

    dispatchCanvasClickIntent(
      intent,
      false,
      actions,
      PLAYER_ATTACK_RANGE_METERS
    )

    expect(actions.breakProp).toHaveBeenCalledWith(intent)
    expect(actions.openProp).not.toHaveBeenCalled()
  })

  it('routes an open_prop intent to openProp', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'open_prop',
      entranceId: 'd1',
      depth: 2,
      propId: 5,
      position: { x: 1, y: 0, z: 2 },
    }

    dispatchCanvasClickIntent(
      intent,
      false,
      actions,
      PLAYER_ATTACK_RANGE_METERS
    )

    expect(actions.openProp).toHaveBeenCalledWith(intent)
    expect(actions.breakProp).not.toHaveBeenCalled()
  })
})

describe('dispatchCanvasClickIntent walk-up interactions', () => {
  it('routes a ground item to pickupItem at any distance', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'pickup_ground_item',
      instanceId: 42,
      position: { x: 1, y: 0, z: 2 },
    }

    dispatchCanvasClickIntent(
      intent,
      false,
      actions,
      PLAYER_ATTACK_RANGE_METERS
    )

    expect(actions.pickupItem).toHaveBeenCalledWith(intent)
  })

  it('routes a far door to toggleDoor rather than a plain walk', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'toggle_door',
      houseId: 'h1',
      roomIndex: 0,
      wallDir: 'north',
      segmentIndex: 2,
      position: { x: 30, y: 0, z: 0 },
    }

    dispatchCanvasClickIntent(
      intent,
      false,
      actions,
      PLAYER_ATTACK_RANGE_METERS
    )

    expect(actions.toggleDoor).toHaveBeenCalledWith(intent)
    expect(actions.moveToGround).not.toHaveBeenCalled()
  })
})

describe('dispatchCanvasClickIntent ground movement', () => {
  it('marks ordinary ground movement as an implicit floor route', () => {
    const actions = makeActions()
    const position = { x: 1, y: 0, z: 2 }

    dispatchCanvasClickIntent(
      { type: 'move_to_ground', position, sprinting: false },
      false,
      actions,
      PLAYER_ATTACK_RANGE_METERS
    )

    expect(actions.moveToGround).toHaveBeenCalledWith(position, false, false)
  })

  it('preserves an explicit housing stair route', () => {
    const actions = makeActions()
    const position = { x: 1, y: 3, z: 2 }

    dispatchCanvasClickIntent(
      {
        type: 'move_to_ground',
        position,
        sprinting: false,
        viaHousingStair: true,
      },
      false,
      actions,
      PLAYER_ATTACK_RANGE_METERS
    )

    expect(actions.moveToGround).toHaveBeenCalledWith(position, false, true)
  })
})

describe('dispatchCanvasClickIntent meals', () => {
  it('routes a meal intent to eatMeal', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'meal',
      mealId: 3,
      position: { x: 1, y: 0.75, z: 2 },
    }

    dispatchCanvasClickIntent(
      intent,
      false,
      actions,
      PLAYER_ATTACK_RANGE_METERS
    )

    expect(actions.eatMeal).toHaveBeenCalledWith(intent)
  })
})

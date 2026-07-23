import { describe, expect, it, vi } from 'vitest'
import type { ClickIntent } from '../../managers/inputHandler'
import { PLAYER_PICKUP_RANGE_METERS } from '../../data/combatTiming'
import {
  dispatchCanvasClickIntent,
  type CanvasClickActions,
} from './canvas-click-dispatcher'

function makeActions() {
  return {
    attackInRange: vi.fn(),
    chaseAndAttack: vi.fn(),
    toggleDoor: vi.fn(),
    toggleDungeonDoor: vi.fn(),
    enterInteraction: vi.fn(),
    enterPickup: vi.fn(),
    approachAndPickup: vi.fn(),
    interactNpc: vi.fn(),
    breakProp: vi.fn(),
    openProp: vi.fn(),
    moveToGround: vi.fn(),
    castFishing: vi.fn(),
  } satisfies CanvasClickActions
}

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

    dispatchCanvasClickIntent(intent, false, actions)

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

    dispatchCanvasClickIntent(intent, false, actions)

    expect(actions.openProp).toHaveBeenCalledWith(intent)
    expect(actions.breakProp).not.toHaveBeenCalled()
  })
})

describe('dispatchCanvasClickIntent pickup handling', () => {
  it('starts pickup immediately when the ground item is within pickup range', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'pickup_ground_item',
      instanceId: 42,
      position: { x: 1, y: 0, z: 2 },
      distance: PLAYER_PICKUP_RANGE_METERS,
    }

    dispatchCanvasClickIntent(intent, false, actions)

    expect(actions.enterPickup).toHaveBeenCalledWith(intent)
    expect(actions.approachAndPickup).not.toHaveBeenCalled()
  })

  it('moves toward the ground item before pickup when it is out of range', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'pickup_ground_item',
      instanceId: 42,
      position: { x: 1, y: 0, z: 2 },
      distance: PLAYER_PICKUP_RANGE_METERS + 0.01,
    }

    dispatchCanvasClickIntent(intent, false, actions)

    expect(actions.approachAndPickup).toHaveBeenCalledWith(intent)
    expect(actions.enterPickup).not.toHaveBeenCalled()
  })
})

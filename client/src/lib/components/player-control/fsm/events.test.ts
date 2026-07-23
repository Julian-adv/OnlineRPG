import { describe, expect, it, vi } from 'vitest'
import type { ClickIntent } from '../../../managers/inputHandler'
import {
  createCanvasIntentEvent,
  dispatchPlayerControlEvent,
  type PlayerControlEvent,
  type PlayerControlEventActions,
} from './events'

function mouseEvent(button: number) {
  return { button } as MouseEvent
}

describe('createCanvasIntentEvent', () => {
  it('uses left click in play mode and right click in editor mode', () => {
    const processIntent = vi.fn(() => ({ type: 'none' as const }))

    expect(
      createCanvasIntentEvent({
        event: mouseEvent(0),
        editorMode: false,
        currentPlayer: { health: 10 },
        processIntent,
      })
    ).toEqual({
      type: 'canvas_intent',
      intent: { type: 'none' },
      editorMode: false,
    })

    expect(
      createCanvasIntentEvent({
        event: mouseEvent(0),
        editorMode: true,
        currentPlayer: { health: 10 },
        processIntent,
      })
    ).toBeNull()

    const editorEvent = createCanvasIntentEvent({
      event: mouseEvent(2),
      editorMode: true,
      currentPlayer: { health: 10 },
      processIntent,
    })
    expect(editorEvent?.type).toBe('canvas_intent')
    if (editorEvent?.type !== 'canvas_intent') return
    expect(editorEvent.editorMode).toBe(true)
  })

  it('ignores missing or dead players without processing intent', () => {
    const processIntent = vi.fn(() => ({ type: 'none' as const }))

    expect(
      createCanvasIntentEvent({
        event: mouseEvent(0),
        editorMode: false,
        currentPlayer: null,
        processIntent,
      })
    ).toBeNull()
    expect(
      createCanvasIntentEvent({
        event: mouseEvent(0),
        editorMode: false,
        currentPlayer: { health: 0 },
        processIntent,
      })
    ).toBeNull()
    expect(processIntent).not.toHaveBeenCalled()
  })
})

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
    requestMove: vi.fn(),
    onInteractionFinished: vi.fn(),
    onPickupGrab: vi.fn(),
    onInteractionRejected: vi.fn(),
  } satisfies PlayerControlEventActions
}

describe('dispatchPlayerControlEvent', () => {
  it('routes request_move events to requestMove', () => {
    const actions = makeActions()
    const event: PlayerControlEvent = {
      type: 'request_move',
      position: { x: 1, y: 2, z: 3 },
      pickupAfterArrival: 99,
    }

    dispatchPlayerControlEvent(event, actions)

    expect(actions.requestMove).toHaveBeenCalledWith(
      { x: 1, y: 2, z: 3 },
      { pickupAfterArrival: 99 }
    )
  })

  it('routes animation events', () => {
    const actions = makeActions()

    dispatchPlayerControlEvent({ type: 'anim_pickup_grab' }, actions)
    dispatchPlayerControlEvent({ type: 'anim_interaction_finished' }, actions)

    expect(actions.onPickupGrab).toHaveBeenCalledOnce()
    expect(actions.onInteractionFinished).toHaveBeenCalledOnce()
  })

  it('delegates canvas intents through the canvas dispatcher', () => {
    const actions = makeActions()
    const intent: ClickIntent = {
      type: 'move_to_ground',
      position: { x: 4, y: 5, z: 6 },
    }

    dispatchPlayerControlEvent(
      { type: 'canvas_intent', intent, editorMode: false },
      actions
    )

    expect(actions.moveToGround).toHaveBeenCalledWith({ x: 4, y: 5, z: 6 })
  })
})

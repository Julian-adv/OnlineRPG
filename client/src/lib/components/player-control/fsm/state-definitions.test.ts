import { describe, expect, it, vi } from 'vitest'
import type { ControlState } from './control-state'
import {
  composePlayerControlStateOverrides,
  createAnimationEventStateOverrides,
  createFramePhaseStateOverrides,
  createLocalPlayerControlMachine,
  createLocalPlayerControlStateDefinitions,
  createNetworkEventStateOverrides,
  createPlayerControlStateDefinitions,
  createTimerCleanupStateOverrides,
} from './state-definitions'

describe('createPlayerControlStateDefinitions', () => {
  it('creates a definition for every control state', () => {
    const states = createPlayerControlStateDefinitions()

    expect(Object.keys(states).sort()).toEqual([
      'attacking',
      'dead',
      'idle',
      'jump_feedback',
      'keyboard_moving',
      'moving',
      'object_interacting',
      'picking_up',
    ])
  })

  it('keeps per-state lifecycle overrides', () => {
    const enter = vi.fn()
    const exit = vi.fn()
    const states = createPlayerControlStateDefinitions({
      moving: { enter, exit },
    })

    states.moving.enter?.()
    states.moving.exit?.()

    expect(enter).toHaveBeenCalledOnce()
    expect(exit).toHaveBeenCalledOnce()
  })
})

describe('composePlayerControlStateOverrides', () => {
  it('chains lifecycle hooks for the same state', () => {
    const calls: string[] = []
    const overrides = composePlayerControlStateOverrides(
      { moving: { enter: () => calls.push('enter:first') } },
      { moving: { enter: () => calls.push('enter:second') } }
    )

    overrides.moving?.enter?.()

    expect(calls).toEqual(['enter:first', 'enter:second'])
  })

  it('runs exit hooks in reverse composition order', () => {
    const calls: string[] = []
    const overrides = composePlayerControlStateOverrides(
      { moving: { exit: () => calls.push('exit:first') } },
      { moving: { exit: () => calls.push('exit:second') } }
    )

    overrides.moving?.exit?.()

    expect(calls).toEqual(['exit:second', 'exit:first'])
  })

  it('falls through phase handlers until one consumes the phase', () => {
    const first = vi.fn(() => false)
    const second = vi.fn(() => true)
    const third = vi.fn(() => true)
    const overrides = composePlayerControlStateOverrides(
      { idle: { handleKeyboard: first } },
      { idle: { handleKeyboard: second } },
      { idle: { handleKeyboard: third } }
    )

    expect(overrides.idle?.handleKeyboard?.(16)).toBe(true)
    expect(first).toHaveBeenCalledOnce()
    expect(second).toHaveBeenCalledOnce()
    expect(third).not.toHaveBeenCalled()
  })
})

describe('createAnimationEventStateOverrides', () => {
  it('routes pickup animation events to pickup actions', () => {
    const onInteractionFinished = vi.fn()
    const onPickupGrab = vi.fn()
    const overrides = createAnimationEventStateOverrides({
      onInteractionFinished,
      onPickupGrab,
    })

    expect(
      overrides.picking_up?.handleEvent?.({ type: 'anim_pickup_grab' })
    ).toBe(true)
    expect(
      overrides.picking_up?.handleEvent?.({
        type: 'anim_interaction_finished',
      })
    ).toBe(true)

    expect(onPickupGrab).toHaveBeenCalledOnce()
    expect(onInteractionFinished).toHaveBeenCalledOnce()
  })

  it('leaves object interaction anim events to the fallback dispatcher', () => {
    const overrides = createAnimationEventStateOverrides({
      onInteractionFinished: vi.fn(),
      onPickupGrab: vi.fn(),
    })

    // No override: the machine falls through to dispatchPlayerControlEvent,
    // which routes anim_interaction_finished to the handler.
    expect(overrides.object_interacting).toBeUndefined()
  })
})

describe('createFramePhaseStateOverrides', () => {
  it('gives every state the default frame phases', () => {
    const overrides = createFramePhaseStateOverrides({
      handleInteractKey: vi.fn(),
      handleKeyboard: vi.fn(),
      tick: vi.fn(),
    })

    expect(Object.keys(overrides).sort()).toEqual([
      'attacking',
      'dead',
      'idle',
      'jump_feedback',
      'keyboard_moving',
      'moving',
      'object_interacting',
      'picking_up',
    ])
  })

  it('consumes phases after calling the injected actions', () => {
    const handleInteractKey = vi.fn()
    const handleKeyboard = vi.fn()
    const tick = vi.fn()
    const overrides = createFramePhaseStateOverrides({
      handleInteractKey,
      handleKeyboard,
      tick,
    })

    expect(overrides.moving?.handleInteractKey?.()).toBe(true)
    expect(overrides.moving?.handleKeyboard?.(16)).toBe(true)
    expect(overrides.moving?.tick?.(16)).toBe(true)

    expect(handleInteractKey).toHaveBeenCalledOnce()
    expect(handleKeyboard).toHaveBeenCalledExactlyOnceWith(16)
    expect(tick).toHaveBeenCalledWith(16)
  })
})

describe('createNetworkEventStateOverrides', () => {
  it('routes interaction rejection through the object interaction state', () => {
    const onInteractionRejected = vi.fn()
    const overrides = createNetworkEventStateOverrides({
      onInteractionRejected,
    })

    expect(
      overrides.object_interacting?.handleEvent?.({
        type: 'network_interaction_rejected',
      })
    ).toBe(true)
    expect(onInteractionRejected).toHaveBeenCalledOnce()
  })

  it('does not consume unrelated network events', () => {
    const overrides = createNetworkEventStateOverrides({
      onInteractionRejected: vi.fn(),
    })

    expect(
      overrides.dead?.handleEvent?.({ type: 'network_interaction_rejected' })
    ).toBeUndefined()
    expect(
      overrides.object_interacting?.handleEvent?.({ type: 'anim_pickup_grab' })
    ).toBeUndefined()
  })
})

describe('createTimerCleanupStateOverrides', () => {
  it('clears the jump feedback timer when leaving jump feedback', () => {
    const clearJumpFeedbackTimer = vi.fn()
    const overrides = createTimerCleanupStateOverrides({
      clearJumpFeedbackTimer,
    })

    overrides.jump_feedback?.exit?.()

    expect(clearJumpFeedbackTimer).toHaveBeenCalledOnce()
  })
})

describe('createLocalPlayerControlStateDefinitions', () => {
  it('wires local player event, timer, network, and frame state behavior', () => {
    const actions = {
      onInteractionFinished: vi.fn(),
      onPickupGrab: vi.fn(),
      clearJumpFeedbackTimer: vi.fn(),
      onInteractionRejected: vi.fn(),
      handleInteractKey: vi.fn(),
      handleKeyboard: vi.fn(),
      tick: vi.fn(),
    }
    const states = createLocalPlayerControlStateDefinitions(actions)

    expect(states.picking_up.handleEvent?.({ type: 'anim_pickup_grab' })).toBe(
      true
    )
    expect(
      states.object_interacting.handleEvent?.({
        type: 'network_interaction_rejected',
      })
    ).toBe(true)
    expect(states.jump_feedback.exit).toBeDefined()
    expect(states.moving.tick?.(16)).toBe(true)

    expect(actions.onPickupGrab).toHaveBeenCalledOnce()
    expect(actions.onInteractionRejected).toHaveBeenCalledOnce()
    expect(actions.tick).toHaveBeenCalledWith(16)
  })

  it('assigns frame phases to every local player control state', () => {
    const actions = {
      onInteractionFinished: vi.fn(),
      onPickupGrab: vi.fn(),
      clearJumpFeedbackTimer: vi.fn(),
      onInteractionRejected: vi.fn(),
      handleInteractKey: vi.fn(),
      handleKeyboard: vi.fn(),
      tick: vi.fn(),
    }
    const states = createLocalPlayerControlStateDefinitions(actions)

    for (const state of Object.values(states)) {
      expect(state.handleInteractKey).toBeDefined()
      expect(state.handleKeyboard).toBeDefined()
      expect(state.tick).toBeDefined()
    }
  })
})

describe('createLocalPlayerControlMachine', () => {
  it('creates a machine wired with local player state behavior', () => {
    const onPickupGrab = vi.fn()
    const tick = vi.fn()
    const machine = createLocalPlayerControlMachine({
      dispatchEvent: vi.fn(),
      stateActions: {
        onInteractionFinished: vi.fn(),
        onPickupGrab,
        clearJumpFeedbackTimer: vi.fn(),
        onInteractionRejected: vi.fn(),
        handleInteractKey: vi.fn(),
        handleKeyboard: vi.fn(),
        tick,
      },
    })

    // The picking_up state's handleEvent consumes anim_pickup_grab → onPickupGrab.
    machine.transition({ name: 'picking_up' } as ControlState)
    machine.update(16, {
      editorMode: false,
      events: [{ type: 'anim_pickup_grab' }],
    })

    expect(onPickupGrab).toHaveBeenCalledOnce()
    expect(tick).toHaveBeenCalledWith(16)
  })
})

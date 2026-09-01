import { describe, expect, it, vi } from 'vitest'
import { DEFAULT_MOVEMENT_CONFIG } from '../../../utils/movementUtils'
import {
  applyKeyboardMovement,
  applyKeyboardMovementOutcome,
  createKeyboardMoveSender,
  createKeyboardSpeedRamp,
  createKeyboardTapTracker,
  runKeyboardFrame,
  KEYBOARD_TAP_STEP,
  type KeyboardFrameActions,
  type KeyboardMovementOutcomeActions,
} from './keyboard'

function makeInput() {
  return {
    currentPos: { x: 0, y: 1, z: 0 },
    direction: { x: 1, z: 0 },
    config: DEFAULT_MOVEMENT_CONFIG,
    deltaTimeSeconds: 1 / 64,
    speedRamp: makeSpeedRamp(),
    sampleHeight: vi.fn((x: number, z: number) => x + z),
    isMovementBlocked: vi.fn(() => false),
    isUphillTooSteep: vi.fn(() => false),
    writePlayerPosition: vi.fn(),
    sendPlayerMove: vi.fn(),
  }
}

function outcomeActions(): KeyboardMovementOutcomeActions {
  return {
    stopMovement: vi.fn(),
    triggerJumpFeedback: vi.fn(),
    setMoved: vi.fn(),
  }
}

function frameActions() {
  return {
    exitPickupInteraction: vi.fn(),
    exitObjectInteraction: vi.fn(),
    clearClickMovement: vi.fn(),
    cancelCombat: vi.fn(),
    markMoving: vi.fn(),
    setKeyboardIdleRuntime: vi.fn(),
    emitKeyboardPlayerState: vi.fn(),
    stopMovement: vi.fn(),
    triggerJumpFeedback: vi.fn(),
    setMoved: vi.fn(),
    requestMove: vi.fn(),
  } satisfies KeyboardFrameActions
}

function makeTapTracker(target: { x: number; z: number } | null = null) {
  return { track: vi.fn(), release: vi.fn(() => target) }
}

function makeMoveSender() {
  return { step: vi.fn(), flush: vi.fn(), reset: vi.fn() }
}

function makeSpeedRamp(speed = 3) {
  return { advance: vi.fn(() => speed), reset: vi.fn() }
}

const movementDeps = {
  config: {
    maxSpeed: 3,
    acceleration: 6,
    deceleration: 6,
    arrivalThreshold: 0.05,
  },
  deltaTimeSeconds: 1 / 64,
  isKeyboardMoving: false,
  sampleHeight: () => 0,
  isMovementBlocked: () => false,
  isUphillTooSteep: () => false,
  writePlayerPosition: vi.fn(),
  tapTracker: makeTapTracker(),
  speedRamp: makeSpeedRamp(),
}

describe('applyKeyboardMovement', () => {
  it('moves by maxSpeed × frame delta and sends the new position', () => {
    const input = makeInput()

    const outcome = applyKeyboardMovement(input)

    expect(outcome.kind).toBe('moved')
    expect(input.writePlayerPosition).toHaveBeenCalledWith(
      { x: 0.046875, y: 0.046875, z: 0 },
      Math.PI / 2
    )
    expect(input.sendPlayerMove).toHaveBeenCalledWith(
      { x: 0.046875, y: 0.046875, z: 0 },
      Math.PI / 2
    )
  })

  it('clamps oversized frame deltas to a 100ms step', () => {
    const input = makeInput()
    input.deltaTimeSeconds = 1

    const outcome = applyKeyboardMovement(input)

    expect(outcome.kind).toBe('moved')
    expect(input.writePlayerPosition).toHaveBeenCalledWith(
      { x: 3 * 0.1, y: 3 * 0.1, z: 0 },
      Math.PI / 2
    )
  })

  it('advances the speed ramp with the clamped frame delta', () => {
    const input = makeInput()
    input.deltaTimeSeconds = 1

    applyKeyboardMovement(input)

    expect(input.speedRamp.advance).toHaveBeenCalledExactlyOnceWith(
      DEFAULT_MOVEMENT_CONFIG,
      0.1
    )
  })

  it('blocks movement before writing or sending', () => {
    const input = makeInput()
    input.isMovementBlocked.mockReturnValue(true)

    const outcome = applyKeyboardMovement(input)

    expect(outcome.kind).toBe('blocked')
    expect(input.writePlayerPosition).not.toHaveBeenCalled()
    expect(input.sendPlayerMove).not.toHaveBeenCalled()
  })

  it('reports steep uphill feedback before writing or sending', () => {
    const input = makeInput()
    input.isUphillTooSteep.mockReturnValue(true)

    const outcome = applyKeyboardMovement(input)

    expect(outcome.kind).toBe('slope_blocked')
    expect(input.writePlayerPosition).not.toHaveBeenCalled()
    expect(input.sendPlayerMove).not.toHaveBeenCalled()
  })
})

describe('createKeyboardSpeedRamp', () => {
  it('accelerates to maxSpeed and restarts after reset', () => {
    const ramp = createKeyboardSpeedRamp()
    const config = {
      maxSpeed: 3,
      acceleration: 6,
      deceleration: 6,
      arrivalThreshold: 0.05,
    }

    expect(ramp.advance(config, 0.25)).toBeCloseTo(1.5)
    expect(ramp.advance(config, 0.25)).toBeCloseTo(3)
    expect(ramp.advance(config, 0.25)).toBeCloseTo(3)

    ramp.reset()
    expect(ramp.advance(config, 0.25)).toBeCloseTo(1.5)
  })
})

describe('applyKeyboardMovementOutcome', () => {
  it('stops movement on blocked outcomes', () => {
    const a = outcomeActions()

    expect(applyKeyboardMovementOutcome({ kind: 'blocked' }, a)).toEqual({
      kind: 'handled',
    })

    expect(a.stopMovement).toHaveBeenCalledOnce()
    expect(a.triggerJumpFeedback).not.toHaveBeenCalled()
  })

  it('stops movement and triggers jump feedback on slope blocks', () => {
    const a = outcomeActions()

    expect(applyKeyboardMovementOutcome({ kind: 'slope_blocked' }, a)).toEqual({
      kind: 'handled',
    })

    expect(a.stopMovement).toHaveBeenCalledOnce()
    expect(a.triggerJumpFeedback).toHaveBeenCalledOnce()
  })

  it('stores moved speed and rotation', () => {
    const a = outcomeActions()

    expect(
      applyKeyboardMovementOutcome(
        { kind: 'moved', currentSpeed: 3, playerRotation: 0.75 },
        a
      )
    ).toEqual({ kind: 'moved' })

    expect(a.setMoved).toHaveBeenCalledWith(3, 0.75)
  })
})

describe('runKeyboardFrame', () => {
  it('does nothing without a player or pressed keys', () => {
    const a = frameActions()
    const moveSender = makeMoveSender()

    runKeyboardFrame({
      currentPlayer: null,
      hasKeysPressed: true,
      interactionExit: 'none',
      hasMovementTarget: false,
      isInCombat: false,
      direction: null,
      actions: a,
      ...movementDeps,
      moveSender,
    })

    expect(a.emitKeyboardPlayerState).not.toHaveBeenCalled()
    expect(moveSender.flush).not.toHaveBeenCalled()
  })

  it('exits interaction and cancels click movement before applying input', () => {
    const a = frameActions()

    runKeyboardFrame({
      currentPlayer: { position: { x: 0, y: 0, z: 0 } },
      hasKeysPressed: true,
      interactionExit: 'object',
      hasMovementTarget: true,
      isInCombat: true,
      direction: null,
      actions: a,
      ...movementDeps,
      moveSender: makeMoveSender(),
    })

    expect(a.exitObjectInteraction).toHaveBeenCalledOnce()
    expect(a.clearClickMovement).toHaveBeenCalledOnce()
    expect(a.cancelCombat).toHaveBeenCalledTimes(2)
    expect(a.setKeyboardIdleRuntime).toHaveBeenCalledOnce()
    expect(a.emitKeyboardPlayerState).toHaveBeenCalledOnce()
  })

  it('marks movement and emits player state after successful movement', () => {
    const a = frameActions()
    const moveSender = makeMoveSender()

    runKeyboardFrame({
      currentPlayer: { position: { x: 0, y: 0, z: 0 } },
      hasKeysPressed: true,
      interactionExit: 'none',
      hasMovementTarget: false,
      isInCombat: false,
      direction: { x: 1, z: 0 },
      actions: a,
      ...movementDeps,
      moveSender,
    })

    expect(a.markMoving).toHaveBeenCalledOnce()
    expect(a.setMoved).toHaveBeenCalledOnce()
    expect(a.emitKeyboardPlayerState).toHaveBeenCalledOnce()
    expect(moveSender.step).toHaveBeenCalledOnce()
  })

  it('flushes the stop position on blocked movement outcomes', () => {
    const a = frameActions()
    const moveSender = makeMoveSender()
    const position = { x: 0, y: 0, z: 0 }

    runKeyboardFrame({
      currentPlayer: { position },
      hasKeysPressed: true,
      interactionExit: 'none',
      hasMovementTarget: false,
      isInCombat: false,
      direction: { x: 1, z: 0 },
      actions: a,
      ...movementDeps,
      moveSender,
      isMovementBlocked: () => true,
    })

    expect(a.stopMovement).toHaveBeenCalledOnce()
    expect(a.emitKeyboardPlayerState).not.toHaveBeenCalled()
    expect(moveSender.flush).toHaveBeenCalledExactlyOnceWith(position)
  })

  it.each([
    ['blocked', { isMovementBlocked: () => true }],
    ['slope-blocked', { isUphillTooSteep: () => true }],
  ])('resets the speed ramp on %s frames', (_label, blocker) => {
    const speedRamp = makeSpeedRamp()

    runKeyboardFrame({
      currentPlayer: { position: { x: 0, y: 0, z: 0 } },
      hasKeysPressed: true,
      interactionExit: 'none',
      hasMovementTarget: false,
      isInCombat: false,
      direction: { x: 1, z: 0 },
      actions: frameActions(),
      ...movementDeps,
      moveSender: makeMoveSender(),
      speedRamp,
      ...blocker,
    })

    expect(speedRamp.reset).toHaveBeenCalledOnce()
  })

  it('flushes the resting position on key release without a click target', () => {
    const a = frameActions()
    const moveSender = makeMoveSender()
    const position = { x: 3, y: 0, z: 4 }

    runKeyboardFrame({
      currentPlayer: { position },
      hasKeysPressed: false,
      interactionExit: 'none',
      hasMovementTarget: false,
      isInCombat: false,
      direction: null,
      actions: a,
      ...movementDeps,
      moveSender,
    })

    expect(moveSender.flush).toHaveBeenCalledExactlyOnceWith(position)
    expect(moveSender.reset).not.toHaveBeenCalled()
  })

  it('settles keyboard_moving to idle on release without a tap target', () => {
    const a = frameActions()
    const moveSender = makeMoveSender()
    const position = { x: 3, y: 0, z: 4 }

    runKeyboardFrame({
      currentPlayer: { position },
      hasKeysPressed: false,
      interactionExit: 'none',
      hasMovementTarget: false,
      isInCombat: false,
      direction: null,
      actions: a,
      ...movementDeps,
      isKeyboardMoving: true,
      moveSender,
    })

    expect(a.setKeyboardIdleRuntime).toHaveBeenCalledOnce()
    expect(a.emitKeyboardPlayerState).toHaveBeenCalledOnce()
    expect(moveSender.flush).toHaveBeenCalledExactlyOnceWith(position)
  })

  it('does not force idle from other states on idle frames', () => {
    const a = frameActions()

    runKeyboardFrame({
      currentPlayer: { position: { x: 3, y: 0, z: 4 } },
      hasKeysPressed: false,
      interactionExit: 'none',
      hasMovementTarget: false,
      isInCombat: false,
      direction: null,
      actions: a,
      ...movementDeps,
      moveSender: makeMoveSender(),
    })

    expect(a.setKeyboardIdleRuntime).not.toHaveBeenCalled()
    expect(a.emitKeyboardPlayerState).not.toHaveBeenCalled()
  })

  it('resets without sending when a click path owns the movement queue', () => {
    const a = frameActions()
    const moveSender = makeMoveSender()

    runKeyboardFrame({
      currentPlayer: { position: { x: 3, y: 0, z: 4 } },
      hasKeysPressed: false,
      interactionExit: 'none',
      hasMovementTarget: true,
      isInCombat: false,
      direction: null,
      actions: a,
      ...movementDeps,
      moveSender,
    })

    expect(moveSender.flush).not.toHaveBeenCalled()
    expect(moveSender.reset).toHaveBeenCalledOnce()
  })

  it('requests a tap-step move on release instead of flushing', () => {
    const a = frameActions()
    const moveSender = makeMoveSender()
    const target = { x: 0.5, z: 0 }

    runKeyboardFrame({
      currentPlayer: { position: { x: 0.05, y: 0, z: 0 } },
      hasKeysPressed: false,
      interactionExit: 'none',
      hasMovementTarget: false,
      isInCombat: false,
      direction: null,
      actions: a,
      ...movementDeps,
      moveSender,
      tapTracker: makeTapTracker(target),
    })

    expect(a.requestMove).toHaveBeenCalledExactlyOnceWith(target)
    expect(moveSender.reset).toHaveBeenCalledOnce()
    expect(moveSender.flush).not.toHaveBeenCalled()
  })

  it('drops the tap target when a click path owns the movement queue', () => {
    const a = frameActions()
    const moveSender = makeMoveSender()

    runKeyboardFrame({
      currentPlayer: { position: { x: 0.05, y: 0, z: 0 } },
      hasKeysPressed: false,
      interactionExit: 'none',
      hasMovementTarget: true,
      isInCombat: false,
      direction: null,
      actions: a,
      ...movementDeps,
      moveSender,
      tapTracker: makeTapTracker({ x: 0.5, z: 0 }),
    })

    expect(a.requestMove).not.toHaveBeenCalled()
    expect(moveSender.reset).toHaveBeenCalledOnce()
  })
})

describe('createKeyboardMoveSender', () => {
  it('replaces on session start, then appends one sample per interval', () => {
    const send = vi.fn()
    const sender = createKeyboardMoveSender(send)

    sender.step({ x: 0.025, y: 0, z: 0 }, 1)
    expect(send).toHaveBeenCalledExactlyOnceWith({ x: 0.025, y: 0, z: 0 }, 1)

    sender.step({ x: 0.2, y: 0, z: 0 }, 1)
    expect(send).toHaveBeenCalledOnce()

    sender.step({ x: 0.6, y: 0, z: 0 }, 1.2)
    expect(send).toHaveBeenCalledTimes(2)
    expect(send).toHaveBeenLastCalledWith(
      { x: 0.6, y: 0, z: 0 },
      1.2,
      undefined,
      true
    )
  })

  it('flush appends the resting position once and ends the session', () => {
    const send = vi.fn()
    const sender = createKeyboardMoveSender(send)

    sender.step({ x: 0.025, y: 0, z: 0 }, 1)
    sender.step({ x: 0.3, y: 0, z: 0 }, 1)
    sender.flush({ x: 0.3, y: 0, z: 0 })

    expect(send).toHaveBeenCalledTimes(2)
    expect(send).toHaveBeenLastCalledWith(
      { x: 0.3, y: 0, z: 0 },
      1,
      undefined,
      true
    )

    sender.flush({ x: 0.3, y: 0, z: 0 })
    expect(send).toHaveBeenCalledTimes(2)
  })

  it('flush without a session is a no-op', () => {
    const send = vi.fn()
    const sender = createKeyboardMoveSender(send)

    sender.flush({ x: 1, y: 0, z: 1 })

    expect(send).not.toHaveBeenCalled()
  })

  it('skips the flush send when the last sample already matches', () => {
    const send = vi.fn()
    const sender = createKeyboardMoveSender(send)

    sender.step({ x: 0.025, y: 0, z: 0 }, 1)
    sender.flush({ x: 0.025, y: 0, z: 0 })

    expect(send).toHaveBeenCalledOnce()

    sender.step({ x: 0.05, y: 0, z: 0 }, 1)
    expect(send).toHaveBeenCalledTimes(2)
    expect(send).toHaveBeenLastCalledWith({ x: 0.05, y: 0, z: 0 }, 1)
  })
})

describe('createKeyboardTapTracker', () => {
  it('completes a short tap to one full step in the last direction', () => {
    const tracker = createKeyboardTapTracker()
    const dir = { x: 1, z: 0 }

    tracker.track({ x: 0, y: 0, z: 0 }, dir)
    tracker.track({ x: 0.05, y: 0, z: 0 }, dir)

    const target = tracker.release({ x: 0.05, y: 0, z: 0 })
    expect(target).not.toBeNull()
    expect(target!.x).toBeCloseTo(KEYBOARD_TAP_STEP)
    expect(target!.z).toBeCloseTo(0)

    expect(tracker.release({ x: 0.05, y: 0, z: 0 })).toBeNull()
  })

  it('normalizes diagonal directions to the step distance', () => {
    const tracker = createKeyboardTapTracker()
    const dir = { x: 1, z: 1 }

    tracker.track({ x: 0, y: 0, z: 0 }, dir)
    tracker.track({ x: 0.02, y: 0, z: 0.02 }, dir)
    const target = tracker.release({ x: 0.02, y: 0, z: 0.02 })

    expect(target).not.toBeNull()
    const dx = target!.x
    const dz = target!.z
    expect(Math.hypot(dx, dz)).toBeCloseTo(KEYBOARD_TAP_STEP, 2)
  })

  it('does not glide after a session that walked past the step distance', () => {
    const tracker = createKeyboardTapTracker()
    const dir = { x: 1, z: 0 }

    tracker.track({ x: 0, y: 0, z: 0 }, dir)
    tracker.track({ x: 0.8, y: 0, z: 0 }, dir)

    expect(tracker.release({ x: 0.8, y: 0, z: 0 })).toBeNull()
  })

  it('does not glide when the session never moved', () => {
    const tracker = createKeyboardTapTracker()

    tracker.track({ x: 3, y: 0, z: 4 }, { x: 0, z: 1 })
    tracker.track({ x: 3, y: 0, z: 4 }, { x: 0, z: 1 })

    expect(tracker.release({ x: 3, y: 0, z: 4 })).toBeNull()
  })
})

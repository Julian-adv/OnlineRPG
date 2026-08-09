import type { PlayerControlEvent } from './events'
import { PlayerControlMachine } from './machine'
import type { PlayerControlStateName } from './control-state'

// ───────────────────────────────────────────────────────────────────────────
// State definition shape + registry
// ───────────────────────────────────────────────────────────────────────────

export interface PlayerControlStateDefinition {
  name: PlayerControlStateName
  enter?: () => void
  exit?: () => void
  handleEvent?: (event: PlayerControlEvent) => boolean | void
  handleInteractKey?: () => boolean | void
  handleKeyboard?: (deltaTime: number) => boolean | void
  tick?: (deltaTime: number) => boolean | void
}

export type PlayerControlStateDefinitions = Record<
  PlayerControlStateName,
  PlayerControlStateDefinition
>

export type PlayerControlStateOverrides = Partial<
  Record<PlayerControlStateName, Omit<PlayerControlStateDefinition, 'name'>>
>

export function createPlayerControlStateDefinitions(
  overrides: PlayerControlStateOverrides = {}
): PlayerControlStateDefinitions {
  return {
    idle: { name: 'idle', ...overrides.idle },
    moving: { name: 'moving', ...overrides.moving },
    keyboard_moving: { name: 'keyboard_moving', ...overrides.keyboard_moving },
    attacking: { name: 'attacking', ...overrides.attacking },
    object_interacting: {
      name: 'object_interacting',
      ...overrides.object_interacting,
    },
    picking_up: { name: 'picking_up', ...overrides.picking_up },
    dead: { name: 'dead', ...overrides.dead },
    jump_feedback: { name: 'jump_feedback', ...overrides.jump_feedback },
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Override composition (merge multiple override sets onto the same state)
// ───────────────────────────────────────────────────────────────────────────

export function composePlayerControlStateOverrides(
  ...overrideSets: PlayerControlStateOverrides[]
): PlayerControlStateOverrides {
  const composed: PlayerControlStateOverrides = {}

  for (const overrides of overrideSets) {
    for (const [stateName, next] of Object.entries(overrides) as Array<
      [PlayerControlStateName, Omit<PlayerControlStateDefinition, 'name'>]
    >) {
      const previous = composed[stateName]
      composed[stateName] = previous
        ? mergeStateOverrides(previous, next)
        : next
    }
  }

  return composed
}

function mergeStateOverrides(
  previous: Omit<PlayerControlStateDefinition, 'name'>,
  next: Omit<PlayerControlStateDefinition, 'name'>
): Omit<PlayerControlStateDefinition, 'name'> {
  return {
    enter: sequence(previous.enter, next.enter),
    exit: sequence(next.exit, previous.exit),
    handleEvent:
      previous.handleEvent || next.handleEvent
        ? (event) =>
            previous.handleEvent?.(event) === true ||
            next.handleEvent?.(event) === true
        : undefined,
    handleInteractKey:
      previous.handleInteractKey || next.handleInteractKey
        ? () =>
            previous.handleInteractKey?.() === true ||
            next.handleInteractKey?.() === true
        : undefined,
    handleKeyboard:
      previous.handleKeyboard || next.handleKeyboard
        ? (deltaTime) =>
            previous.handleKeyboard?.(deltaTime) === true ||
            next.handleKeyboard?.(deltaTime) === true
        : undefined,
    tick:
      previous.tick || next.tick
        ? (deltaTime) =>
            previous.tick?.(deltaTime) === true ||
            next.tick?.(deltaTime) === true
        : undefined,
  }
}

function sequence(first?: () => void, second?: () => void) {
  if (!first) return second
  if (!second) return first
  return () => {
    first()
    second()
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Animation event overrides (pickup grab / interaction finished)
// ───────────────────────────────────────────────────────────────────────────

export interface AnimationEventStateActions {
  onInteractionFinished: () => void
  onPickupGrab: () => void
}

export function createAnimationEventStateOverrides({
  onInteractionFinished,
  onPickupGrab,
}: AnimationEventStateActions): PlayerControlStateOverrides {
  return {
    picking_up: {
      handleEvent: (event) => {
        switch (event.type) {
          case 'anim_pickup_grab':
            onPickupGrab()
            return true
          case 'anim_interaction_finished':
            onInteractionFinished()
            return true
        }
      },
    },
    // object_interacting has no override: anim_interaction_finished falls
    // through to the shared dispatcher, whose handler exits one-shot emotes
    // and ignores held poses (bench, forge).
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Frame phase overrides (interact key / keyboard / tick on every state)
// ───────────────────────────────────────────────────────────────────────────

export interface FramePhaseStateActions {
  handleInteractKey: () => void
  handleKeyboard: (deltaTime: number) => void
  tick: (deltaTime: number) => void
}

const framePhaseStateNames = [
  'idle',
  'moving',
  'keyboard_moving',
  'attacking',
  'object_interacting',
  'picking_up',
  'dead',
  'jump_feedback',
] as const satisfies readonly PlayerControlStateName[]

export function createFramePhaseStateOverrides({
  handleInteractKey,
  handleKeyboard,
  tick,
}: FramePhaseStateActions): PlayerControlStateOverrides {
  return Object.fromEntries(
    framePhaseStateNames.map((stateName) => [
      stateName,
      {
        handleInteractKey: () => {
          handleInteractKey()
          return true
        },
        handleKeyboard: (deltaTime: number) => {
          handleKeyboard(deltaTime)
          return true
        },
        tick: (deltaTime: number) => {
          tick(deltaTime)
          return true
        },
      },
    ])
  )
}

// ───────────────────────────────────────────────────────────────────────────
// Network event overrides (interaction rejected)
// ───────────────────────────────────────────────────────────────────────────

export interface NetworkEventStateActions {
  onInteractionRejected: () => void
}

export function createNetworkEventStateOverrides({
  onInteractionRejected,
}: NetworkEventStateActions): PlayerControlStateOverrides {
  return {
    object_interacting: {
      handleEvent: (event) => {
        if (event.type !== 'network_interaction_rejected') return
        onInteractionRejected()
        return true
      },
    },
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Timer cleanup overrides (jump feedback timer on exit)
// ───────────────────────────────────────────────────────────────────────────

export interface TimerCleanupStateActions {
  clearJumpFeedbackTimer: () => void
}

export function createTimerCleanupStateOverrides({
  clearJumpFeedbackTimer,
}: TimerCleanupStateActions): PlayerControlStateOverrides {
  return {
    jump_feedback: {
      exit: clearJumpFeedbackTimer,
    },
  }
}

// ───────────────────────────────────────────────────────────────────────────
// Local player control state table + machine factory
// ───────────────────────────────────────────────────────────────────────────

export interface LocalPlayerControlStateActions
  extends
    AnimationEventStateActions,
    TimerCleanupStateActions,
    NetworkEventStateActions,
    FramePhaseStateActions {}

export function createLocalPlayerControlStateDefinitions(
  actions: LocalPlayerControlStateActions
) {
  return createPlayerControlStateDefinitions(
    composePlayerControlStateOverrides(
      createAnimationEventStateOverrides(actions),
      createTimerCleanupStateOverrides(actions),
      createNetworkEventStateOverrides(actions),
      createFramePhaseStateOverrides(actions)
    )
  )
}

interface CreateLocalPlayerControlMachineInput {
  dispatchEvent: (event: PlayerControlEvent) => void
  stateActions: LocalPlayerControlStateActions
}

export function createLocalPlayerControlMachine({
  dispatchEvent,
  stateActions,
}: CreateLocalPlayerControlMachineInput) {
  return new PlayerControlMachine(
    {
      dispatchEvent,
    },
    {
      states: createLocalPlayerControlStateDefinitions(stateActions),
    }
  )
}

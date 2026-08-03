import { describe, expect, it, vi } from 'vitest'
import { CombatController } from './combatController'

vi.mock('./bgmManager', () => ({
  startBattleMusic: vi.fn(),
  stopBattleMusic: vi.fn(),
}))

describe('CombatController', () => {
  it('re-approaches instead of starting a new attack cycle after the target flees out of range', () => {
    const controller = new CombatController()
    controller.beginCombat('m1', true)

    const result = controller.update(
      1500,
      { x: 0, y: 0, z: 0 },
      { state: 'run' },
      { x: 3.5, y: 0, z: 0 },
      false,
      1500,
      'attack'
    )

    expect(result).toEqual({
      action: 'chasing',
      newTarget: { x: 3.5, y: 0, z: 0 },
    })
    expect(controller.isInCombat).toBe(true)
  })

  it('re-approaches when the target is outside player reach but still near monster reach', () => {
    const controller = new CombatController()
    controller.beginCombat('m1', true)

    const result = controller.update(
      1500,
      { x: 0, y: 0, z: 0 },
      { state: 'attack' },
      { x: 2.5, y: 0, z: 0 },
      false,
      1500,
      'attack'
    )

    expect(result).toEqual({
      action: 'chasing',
      newTarget: { x: 2.5, y: 0, z: 0 },
    })
    expect(controller.isInCombat).toBe(true)
  })

  it('starts the next attack cycle when the target is still in range', () => {
    const controller = new CombatController()
    controller.beginCombat('m1', true)

    const result = controller.update(
      1500,
      { x: 0, y: 0, z: 0 },
      { state: 'idle' },
      { x: 1.5, y: 0, z: 0 },
      false,
      1500,
      'attack'
    )

    expect(result).toEqual({
      action: 'attack_cycle',
      monsterId: 'm1',
      rotation: Math.PI / 2,
    })
  })

  it('uses the supplied Spear reach and cadence for attack cycles', () => {
    const controller = new CombatController()
    controller.beginCombat('m1', true)

    const result = controller.update(
      2467,
      { x: 0, y: 0, z: 0 },
      { state: 'idle' },
      { x: 2.9, y: 0, z: 0 },
      false,
      2467,
      'attack',
      3
    )

    expect(result).toEqual({
      action: 'attack_cycle',
      monsterId: 'm1',
      rotation: Math.PI / 2,
    })
  })
})

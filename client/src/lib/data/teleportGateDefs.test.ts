import { describe, expect, it } from 'vitest'
import {
  getTeleportGate,
  TELEPORT_GATES,
  TELEPORT_GATE_CONFIG,
  TELEPORT_GATE_WARNING,
} from './teleportGateDefs'

describe('town teleport-gate definitions', () => {
  it('registers one uniquely positioned gate for every current town', () => {
    expect(TELEPORT_GATES.map((gate) => gate.id)).toEqual([
      'aldermark',
      'brovik',
      'edra',
      'frihavn',
      'garasden',
      'mistfall',
      'riftmark',
      'stenhavn',
    ])
    expect(
      new Set(TELEPORT_GATES.map((gate) => `${gate.x},${gate.z}`)).size
    ).toBe(TELEPORT_GATES.length)
  })

  it('exposes authored destinations and the player-facing risk notice', () => {
    expect(getTeleportGate('garasden')?.name).toBe('Garasden')
    expect(getTeleportGate('not-a-town')).toBeUndefined()
    expect(TELEPORT_GATE_CONFIG.misfireChanceBps).toBe(50)
    expect(TELEPORT_GATE_CONFIG.dungeonMisfirePercent).toBe(20)
    expect(TELEPORT_GATE_WARNING).toMatch(/distance/i)
    expect(TELEPORT_GATE_WARNING).toMatch(/misfire/i)
    expect(TELEPORT_GATE_WARNING).toMatch(/land, sea, or a dungeon/i)
  })
})

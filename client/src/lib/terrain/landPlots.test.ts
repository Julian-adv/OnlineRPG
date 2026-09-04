import { describe, expect, it } from 'vitest'
import { LandGrade, nextGrade, plotAddress, plotOrigin } from './landPlots'
import { LAND_PLOT_SIZE } from './terrain-constants'

describe('landPlots', () => {
  it('round-trips addresses through origins across boundaries', () => {
    for (const [x, z] of [
      [-33, -33],
      [-32, -32],
      [-1, 0],
      [0, 0],
      [31.9, 31.9],
      [32, 32],
      [1023, -1025],
      [1024, -1024],
    ]) {
      const a = plotAddress(x, z)
      const o = plotOrigin(a.rx, a.rz, a.index)
      expect(o.x <= x && x < o.x + LAND_PLOT_SIZE).toBe(true)
      expect(o.z <= z && z < o.z + LAND_PLOT_SIZE).toBe(true)
    }
  })

  it('splits tile zero into four quadrants', () => {
    expect(plotAddress(-1, -1).index).toBe(0)
    expect(plotAddress(1, -1).index).toBe(1)
    expect(plotAddress(-1, 1).index).toBe(2)
    expect(plotAddress(1, 1).index).toBe(3)
    expect(plotAddress(32, -1).index).toBe(4)
  })

  it('wraps x into canonical regions', () => {
    expect(plotAddress(-16 * 1024 - 33, 0).rx).toBe(15)
    expect(plotAddress(16 * 1024 - 32, 0).rx).toBe(-16)
  })

  it('cycles grades prime → pioneer → nobuild → prime', () => {
    expect(nextGrade(LandGrade.Prime)).toBe(LandGrade.Pioneer)
    expect(nextGrade(LandGrade.Pioneer)).toBe(LandGrade.NoBuild)
    expect(nextGrade(LandGrade.NoBuild)).toBe(LandGrade.Prime)
  })
})

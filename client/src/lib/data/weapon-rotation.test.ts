import { describe, expect, it } from 'vitest'
import { parseWeaponRotation } from './monsterDefs'

const DEG = Math.PI / 180

describe('parseWeaponRotation', () => {
  it('is identity when the column is empty', () => {
    expect(parseWeaponRotation(undefined)).toEqual([0, 0, 0])
    expect(parseWeaponRotation('')).toEqual([0, 0, 0])
  })

  it('reads degrees about each local axis', () => {
    const [x, y, z] = parseWeaponRotation('90|-45|180')
    expect(x).toBeCloseTo(90 * DEG, 10)
    expect(y).toBeCloseTo(-45 * DEG, 10)
    expect(z).toBeCloseTo(180 * DEG, 10)
  })

  it('tolerates spacing and a short list', () => {
    expect(parseWeaponRotation(' 0 | 90 ')[1]).toBeCloseTo(90 * DEG, 10)
    expect(parseWeaponRotation('0|90')[2]).toBe(0)
  })

  // The CSV converter coerces anything numeric, so a lone angle is a number.
  it('takes a bare number as rotation about x', () => {
    expect(parseWeaponRotation(90)[0]).toBeCloseTo(90 * DEG, 10)
  })

  it('drops a component it cannot read rather than producing NaN', () => {
    expect(parseWeaponRotation('90|banana|0')).toEqual([90 * DEG, 0, 0])
  })
})

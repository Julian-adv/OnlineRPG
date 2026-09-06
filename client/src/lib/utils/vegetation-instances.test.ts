import { describe, expect, it } from 'vitest'
import { filterVegetationInstances } from './vegetation-instances'

describe('vegetation instance filtering', () => {
  const instances = new Float32Array([
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
  ])

  it('preserves height, rotation, scale and order for surviving instances', () => {
    const filtered = filterVegetationInstances(
      instances,
      (x, z) => x === 6 && z === 8
    )
    expect(Array.from(filtered)).toEqual([1, 2, 3, 4, 5, 11, 12, 13, 14, 15])
    expect(instances.length).toBe(15)
  })

  it('reuses unchanged data and handles complete removal', () => {
    expect(filterVegetationInstances(instances, () => false)).toBe(instances)
    expect(filterVegetationInstances(instances, () => true).length).toBe(0)
  })
})

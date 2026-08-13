import { describe, expect, it } from 'vitest'
import { parseTpArgs, resolveTpDestination } from './tp-args'

describe('parseTpArgs', () => {
  it('parses x z with y defaulting to 0', () => {
    expect(parseTpArgs('-1450 4720')).toEqual({ x: -1450, z: 4720, y: 0 })
  })

  it('parses an explicit y', () => {
    expect(parseTpArgs('-1443.5 4711.5 -19.3')).toEqual({
      x: -1443.5,
      z: 4711.5,
      y: -19.3,
    })
  })

  it('tolerates extra whitespace', () => {
    expect(parseTpArgs('  10   20  ')).toEqual({ x: 10, z: 20, y: 0 })
  })

  it('rejects wrong argument counts', () => {
    expect(parseTpArgs('')).toBeNull()
    expect(parseTpArgs('5')).toBeNull()
    expect(parseTpArgs('1 2 3 4')).toBeNull()
  })

  it('rejects non-numeric input', () => {
    expect(parseTpArgs('abc 5')).toBeNull()
    expect(parseTpArgs('5 5.5.5')).toBeNull()
  })

  // isNaN() admits Infinity; these poisoned the position with NaN after
  // world-wrap and stalled the server's whole save batch.
  it('rejects non-finite values', () => {
    expect(parseTpArgs('Infinity 0')).toBeNull()
    expect(parseTpArgs('0 -Infinity')).toBeNull()
    expect(parseTpArgs('1e999 0')).toBeNull()
    expect(parseTpArgs('0 0 NaN')).toBeNull()
  })

  it('rejects values that overflow f32 on the wire', () => {
    expect(parseTpArgs('0 1e39')).toBeNull()
    expect(parseTpArgs('1e39 0')).toBeNull()
  })
})

describe('resolveTpDestination', () => {
  const dests = [
    { name: 'spawn', x: 0 },
    { name: 'crypt-boss', x: 1 },
    { name: 'Snowpeak', x: 2 },
  ]

  it('resolves by case-insensitive name', () => {
    expect(resolveTpDestination('crypt-boss', dests)?.x).toBe(1)
    expect(resolveTpDestination('SNOWPEAK', dests)?.x).toBe(2)
  })

  it('resolves by 1-based index', () => {
    expect(resolveTpDestination('1', dests)?.name).toBe('spawn')
    expect(resolveTpDestination('3', dests)?.name).toBe('Snowpeak')
  })

  it('returns null for unknown names and out-of-range indices', () => {
    expect(resolveTpDestination('nowhere', dests)).toBeNull()
    expect(resolveTpDestination('0', dests)).toBeNull()
    expect(resolveTpDestination('4', dests)).toBeNull()
  })
})

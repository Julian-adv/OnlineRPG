import { describe, expect, it } from 'vitest'
import { formatGold, parseGold, splitGold } from './currency'

describe('splitGold', () => {
  it('splits copper into gold/silver/copper', () => {
    expect(splitGold(12_030)).toEqual({
      negative: false,
      gold: 1,
      silver: 20,
      copper: 30,
    })
  })
})

describe('formatGold', () => {
  it('drops empty units and keeps 0c for nothing', () => {
    expect(formatGold(12_030)).toBe('1g 20s 30c')
    expect(formatGold(10_000)).toBe('1g')
    expect(formatGold(105)).toBe('1s 5c')
    expect(formatGold(0)).toBe('0c')
    expect(formatGold(-250)).toBe('-2s 50c')
  })
})

describe('parseGold', () => {
  it('reads what formatGold writes', () => {
    expect(parseGold('1g 20s 30c')).toBe(12_030)
    expect(parseGold('0c')).toBe(0)
  })

  it('reads compact and partial forms, ignoring case and spaces', () => {
    expect(parseGold('1g20s30c')).toBe(12_030)
    expect(parseGold('2S')).toBe(200)
    expect(parseGold(' 1g  5c ')).toBe(10_005)
  })

  it('treats a bare number as copper', () => {
    expect(parseGold('500')).toBe(500)
  })

  it('rejects junk, empty input and repeated units', () => {
    expect(parseGold('')).toBeNull()
    expect(parseGold('abc')).toBeNull()
    expect(parseGold('1g20')).toBeNull()
    expect(parseGold('1g2g')).toBeNull()
    expect(parseGold('30c1g')).toBeNull() // units must read big to small
    expect(parseGold('-5c')).toBeNull()
  })
})

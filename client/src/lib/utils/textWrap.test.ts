import { describe, expect, it } from 'vitest'
import { wrapLines } from './textWrap'

const measure = (s: string) => s.length * 10

describe('wrapLines', () => {
  it('wraps spaced text at word boundaries without splitting words', () => {
    expect(wrapLines('the quick brown fox', 100, false, measure)).toEqual([
      'the quick',
      'brown fox',
    ])
  })

  it('breaks a single unspaced run wider than the line', () => {
    const lines = wrapLines('ㅁ'.repeat(25), 100, false, measure)
    expect(lines).toEqual(['ㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁ', 'ㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁ', 'ㅁㅁㅁㅁㅁ'])
    for (const line of lines) expect(measure(line)).toBeLessThanOrEqual(100)
  })

  it('continues the line after an oversized word is broken', () => {
    expect(wrapLines('ab ㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁ cd', 100, false, measure)).toEqual([
      'ab',
      'ㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁ',
      'ㅁㅁ cd',
    ])
  })

  it('keeps explicit newlines and empty lines', () => {
    expect(wrapLines('a\n\nb', 100, false, measure)).toEqual(['a', '', 'b'])
  })

  it('breaks every character run in break-word mode', () => {
    expect(wrapLines('abcdefghijkl', 50, true, measure)).toEqual([
      'abcde',
      'fghij',
      'kl',
    ])
  })

  it('returns a single empty line for empty input', () => {
    expect(wrapLines('', 100, false, measure)).toEqual([''])
  })

  it('never breaks inside a ZWJ emoji cluster', () => {
    const family = '👨‍👩‍👧‍👦'
    const seg = new Intl.Segmenter(undefined, { granularity: 'grapheme' })
    const glyphs = (s: string) => [...seg.segment(s)].length
    const measureGlyphs = (s: string) => glyphs(s) * 10

    const lines = wrapLines(family.repeat(5), 20, false, measureGlyphs)
    expect(lines).toEqual([
      family + family,
      family + family,
      family,
    ])
  })
})

import { describe, expect, it } from 'vitest'
import { truncateGraphemes, wrapLines } from './textWrap'

const measure = (s: string) => s.length * 10

describe('wrapLines', () => {
  it('wraps spaced text at word boundaries without splitting words', () => {
    expect(wrapLines('the quick brown fox', 100, measure)).toEqual([
      'the quick',
      'brown fox',
    ])
  })

  it('breaks a single unspaced run wider than the line', () => {
    const lines = wrapLines('ㅁ'.repeat(25), 100, measure)
    expect(lines).toEqual([
      'ㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁ',
      'ㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁ',
      'ㅁㅁㅁㅁㅁ',
    ])
    for (const line of lines) expect(measure(line)).toBeLessThanOrEqual(100)
  })

  it('continues the line after an oversized word is broken', () => {
    expect(wrapLines('ab ㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁ cd', 100, measure)).toEqual([
      'ab',
      'ㅁㅁㅁㅁㅁㅁㅁㅁㅁㅁ',
      'ㅁㅁ cd',
    ])
  })

  it('keeps explicit newlines and empty lines', () => {
    expect(wrapLines('a\n\nb', 100, measure)).toEqual(['a', '', 'b'])
  })

  it('returns a single empty line for empty input', () => {
    expect(wrapLines('', 100, measure)).toEqual([''])
  })

  it('never breaks inside a ZWJ emoji cluster', () => {
    const family = '👨‍👩‍👧‍👦'
    const seg = new Intl.Segmenter(undefined, { granularity: 'grapheme' })
    const glyphs = (s: string) => [...seg.segment(s)].length
    const measureGlyphs = (s: string) => glyphs(s) * 10

    const lines = wrapLines(family.repeat(5), 20, measureGlyphs)
    expect(lines).toEqual([family + family, family + family, family])
  })
})

describe('truncateGraphemes', () => {
  it('returns text at or under the limit unchanged', () => {
    expect(truncateGraphemes('abc', 3)).toBe('abc')
    expect(truncateGraphemes('', 3)).toBe('')
  })

  it('truncates without splitting a ZWJ emoji cluster', () => {
    const family = '👨‍👩‍👧‍👦'
    expect(truncateGraphemes('ab' + family + 'cd', 3)).toBe('ab' + family)
  })
})

import { describe, expect, it } from 'vitest'
import { ChatHistory, CHAT_HISTORY_MAX } from './chat-history'

describe('ChatHistory', () => {
  it('walks back through sent lines and forward to the draft', () => {
    const h = new ChatHistory()
    h.push('one')
    h.push('two')
    expect(h.prev('draft')).toBe('two')
    expect(h.prev('two')).toBe('one')
    expect(h.prev('one')).toBeNull()
    expect(h.next('one')).toBe('two')
    expect(h.next('two')).toBe('draft')
    expect(h.next('draft')).toBeNull()
  })

  it('does nothing when empty', () => {
    const h = new ChatHistory()
    expect(h.prev('')).toBeNull()
    expect(h.next('')).toBeNull()
  })

  it('skips consecutive duplicates and blank lines', () => {
    const h = new ChatHistory()
    h.push('hi')
    h.push('hi')
    h.push('')
    expect(h.lines).toEqual(['hi'])
  })

  it('keeps only the newest entries', () => {
    const h = new ChatHistory()
    for (let i = 0; i < CHAT_HISTORY_MAX + 5; i++) h.push(`m${i}`)
    expect(h.lines.length).toBe(CHAT_HISTORY_MAX)
    expect(h.lines[0]).toBe('m5')
  })

  it('restarts from the newest line after a push or an edit', () => {
    const h = new ChatHistory(['a', 'b'])
    expect(h.prev('')).toBe('b')
    expect(h.prev('b')).toBe('a')
    expect(h.next('a-edited')).toBeNull()
    expect(h.prev('a-edited')).toBe('b')
    expect(h.next('b')).toBe('a-edited')
    h.push('c')
    expect(h.prev('')).toBe('c')
  })
})

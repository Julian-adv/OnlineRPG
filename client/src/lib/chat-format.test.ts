import { describe, it, expect } from 'vitest'
import {
  isPartyTabLine,
  unreadPartyCount,
  whisperChatEntry,
} from './chat-format'
import type { ChatSender } from './stores/gameStore'

describe('whisperChatEntry', () => {
  it('labels the echo of an own whisper as outgoing', () => {
    expect(whisperChatEntry('Miru', 'Rica', 'psst', 'Miru')).toEqual({
      text: 'psst',
      sender: 'whisper',
      name: 'To Rica',
    })
  })

  it('labels a received whisper with the sender', () => {
    expect(whisperChatEntry('Miru', 'Rica', 'psst', 'Rica')).toEqual({
      text: 'psst',
      sender: 'whisper',
      name: 'From Miru',
    })
  })

  it('treats an unknown own name as incoming', () => {
    expect(whisperChatEntry('Miru', 'Rica', 'psst', undefined).name).toBe(
      'From Miru'
    )
  })
})

describe('unreadPartyCount', () => {
  const TRANSCRIPT = [
    { sender: 'party' as const, name: 'Rica', id: 1 },
    { sender: 'local' as const, name: 'Miru', id: 2 },
    { sender: 'party' as const, name: 'Rica', id: 3 },
    { sender: 'system' as const, id: 4 },
    { sender: 'party' as const, name: 'Rica', id: 5 },
  ]

  it('counts only the party lines newer than the last one seen', () => {
    expect(unreadPartyCount(TRANSCRIPT, 0, 'Miru')).toBe(3)
    expect(unreadPartyCount(TRANSCRIPT, 3, 'Miru')).toBe(1)
    expect(unreadPartyCount(TRANSCRIPT, 5, 'Miru')).toBe(0)
  })

  it('never counts the echo of an own party line', () => {
    expect(unreadPartyCount(TRANSCRIPT, 0, 'Rica')).toBe(0)
  })

  it('counts everything when the own name is not known yet', () => {
    expect(unreadPartyCount(TRANSCRIPT, 0, undefined)).toBe(3)
  })

  it('survives the ring buffer dropping the entries it counted', () => {
    // Ids only grow, so evicting 1 and 3 leaves the unread 5 counted once.
    expect(unreadPartyCount(TRANSCRIPT.slice(3), 4, 'Miru')).toBe(1)
    expect(unreadPartyCount([], 4, 'Miru')).toBe(0)
  })
})

describe('isPartyTabLine', () => {
  const line = (sender: ChatSender, text: string) => ({ sender, text })

  it('takes the party channel and its system notices', () => {
    expect(isPartyTabLine(line('party', 'on my way'))).toBe(true)
    expect(isPartyTabLine(line('system', 'Party: Rica joined.'))).toBe(true)
    expect(isPartyTabLine(line('system', 'Summon: Rica calls you.'))).toBe(true)
  })

  it('leaves every other line to the All tab', () => {
    expect(isPartyTabLine(line('local', 'Party: hi'))).toBe(false)
    expect(isPartyTabLine(line('whisper', 'psst'))).toBe(false)
    expect(isPartyTabLine(line('system', 'Muted: 10 minutes left.'))).toBe(
      false
    )
  })
})

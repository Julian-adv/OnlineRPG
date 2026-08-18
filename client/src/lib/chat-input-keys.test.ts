import { describe, expect, it } from 'vitest'
import {
  chatInputKeyIntent,
  commandCompletions,
  shouldFocusChatOnEnter,
} from './chat-input-keys'

function key(
  key: string,
  opts: { isComposing?: boolean; keyCode?: number } = {}
) {
  return {
    key,
    isComposing: opts.isComposing ?? false,
    keyCode: opts.keyCode ?? 0,
  }
}

describe('chatInputKeyIntent', () => {
  it('sends on plain Enter', () => {
    expect(chatInputKeyIntent(key('Enter', { keyCode: 13 }))).toBe('send')
  })

  it('does not send on Enter during IME composition', () => {
    expect(
      chatInputKeyIntent(key('Enter', { isComposing: true, keyCode: 13 }))
    ).toBe('none')
  })

  it('does not send on IME keydown reported as keyCode 229', () => {
    expect(chatInputKeyIntent(key('Enter', { keyCode: 229 }))).toBe('none')
  })

  it('completes commands on Tab', () => {
    expect(chatInputKeyIntent(key('Tab', { keyCode: 9 }))).toBe(
      'complete-command'
    )
  })

  it('browses history on the arrow keys', () => {
    expect(chatInputKeyIntent(key('ArrowUp', { keyCode: 38 }))).toBe(
      'history-prev'
    )
    expect(chatInputKeyIntent(key('ArrowDown', { keyCode: 40 }))).toBe(
      'history-next'
    )
    expect(
      chatInputKeyIntent(key('ArrowUp', { isComposing: true, keyCode: 38 }))
    ).toBe('none')
  })

  it('does not complete commands on Tab during IME composition', () => {
    expect(
      chatInputKeyIntent(key('Tab', { isComposing: true, keyCode: 9 }))
    ).toBe('none')
    expect(chatInputKeyIntent(key('Tab', { keyCode: 229 }))).toBe('none')
  })

  it('ignores other keys', () => {
    expect(chatInputKeyIntent(key('a', { keyCode: 65 }))).toBe('none')
    expect(chatInputKeyIntent(key('Escape', { keyCode: 27 }))).toBe('none')
  })
})

describe('shouldFocusChatOnEnter', () => {
  it('focuses chat on a plain Enter', () => {
    expect(shouldFocusChatOnEnter(key('Enter', { keyCode: 13 }), false)).toBe(
      true
    )
  })

  it('keeps Enter for the channel menu while it is open', () => {
    // Cancelling the keydown would suppress the focused menu item's
    // activation and strand the open menu on screen.
    expect(shouldFocusChatOnEnter(key('Enter', { keyCode: 13 }), true)).toBe(
      false
    )
  })

  it('ignores Enter during IME composition', () => {
    expect(
      shouldFocusChatOnEnter(key('Enter', { isComposing: true }), false)
    ).toBe(false)
    expect(shouldFocusChatOnEnter(key('Enter', { keyCode: 229 }), false)).toBe(
      false
    )
  })

  it('ignores other keys', () => {
    expect(shouldFocusChatOnEnter(key('Escape', { keyCode: 27 }), false)).toBe(
      false
    )
  })
})

describe('commandCompletions', () => {
  // Sorted, like the real `commandNames`: an exact match sorts ahead of the
  // longer names sharing its prefix, which is what made `/p` preview `/party`.
  const NAMES = ['/p', '/party', '/pos', '/s', '/w', '/whisper', '/who']

  it('lists the commands a half-typed one could become', () => {
    expect(commandCompletions('/wh', NAMES)).toEqual(['/whisper', '/who'])
    expect(commandCompletions('/pa', NAMES)).toEqual(['/party'])
  })

  it('completes nothing once the input is itself a command', () => {
    expect(commandCompletions('/p', NAMES)).toEqual([])
    expect(commandCompletions('/s', NAMES)).toEqual([])
    expect(commandCompletions('/w', NAMES)).toEqual([])
  })

  it('leaves plain text and anything past the command word alone', () => {
    expect(commandCompletions('hello', NAMES)).toEqual([])
    expect(commandCompletions('/p meet me', NAMES)).toEqual([])
    expect(commandCompletions('/zz', NAMES)).toEqual([])
  })
})

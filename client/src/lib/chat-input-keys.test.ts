import { describe, expect, it } from 'vitest'
import { chatInputKeyIntent, shouldFocusChatOnEnter } from './chat-input-keys'

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

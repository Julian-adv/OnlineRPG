export type ChatKeyIntent = 'complete-command' | 'send' | 'none'

/** Enter anywhere outside the input focuses chat — but not while the channel
 *  menu is open: cancelling that keydown would swallow the focused menu
 *  item's activation and strand the menu on screen. */
export function shouldFocusChatOnEnter(
  event: { key: string; isComposing: boolean; keyCode: number },
  channelMenuOpen: boolean
): boolean {
  if (event.isComposing || event.keyCode === 229) return false
  return event.key === 'Enter' && !channelMenuOpen
}

/** Keys during IME composition only drive the composition; keyCode 229 covers
 *  browsers that fire IME keydowns without isComposing. Reproduces on macOS
 *  Korean IME only — Windows commits the syllable before dispatching Enter. */
export function chatInputKeyIntent(event: {
  key: string
  isComposing: boolean
  keyCode: number
}): ChatKeyIntent {
  if (event.isComposing || event.keyCode === 229) return 'none'
  if (event.key === 'Tab') return 'complete-command'
  if (event.key !== 'Enter') return 'none'
  return 'send'
}

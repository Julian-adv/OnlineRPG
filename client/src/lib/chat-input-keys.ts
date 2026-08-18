export type ChatKeyIntent =
  | 'complete-command'
  | 'send'
  | 'history-prev'
  | 'history-next'
  | 'none'

/** What Tab cycles through for a half-typed command, ghost preview first. A
 *  name that is already a command completes to nothing: `/p` and `/s` are
 *  commands in their own right, not half-typed `/party` or `/shore_wave`. */
export function commandCompletions(input: string, names: string[]): string[] {
  if (!input.startsWith('/') || input.includes(' ')) return []
  if (names.includes(input)) return []
  return names.filter((name) => name.startsWith(input))
}

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
  if (event.key === 'ArrowUp') return 'history-prev'
  if (event.key === 'ArrowDown') return 'history-next'
  if (event.key !== 'Enter') return 'none'
  return 'send'
}

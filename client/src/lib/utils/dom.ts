/** True when the event targets a text field — keys there are typing (chat,
 *  forms), never gameplay input. */
export function isTypingTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA')
  )
}

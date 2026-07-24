// Combat-log wording for a landed catch, category-aware: fish are caught
// (with their size), junk is fished up, a coin catch announces itself (the
// gold arrives via GoldGained right behind it). Pure so it's unit-testable —
// keep the phrasing in sync with the agent-client's caught_line
// (agent-client/src/driver/prompt.rs).

export interface CatchDefLike {
  name: string
  category?: string
}

export function catchMessage(
  def: CatchDefLike | undefined,
  fallbackId: string,
  sizeCm: number,
  trophy: boolean
): string {
  const name = def?.name ?? fallbackId
  const an = /^[aeiou]/i.test(name) ? 'an' : 'a'
  if (trophy) return `Trophy catch! ${name}, ${sizeCm} cm!`
  if (def?.category === 'coin_catch') return `You haul up ${an} ${name}!`
  if (def?.category === 'fish')
    return `You caught ${an} ${name} (${sizeCm} cm).`
  return `You fished up ${an} ${name}.`
}

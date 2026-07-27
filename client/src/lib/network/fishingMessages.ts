// Combat-log wording for a landed catch. Pure for unit tests; keep in sync
// with agent-client/src/driver/prompt.rs caught_line.

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

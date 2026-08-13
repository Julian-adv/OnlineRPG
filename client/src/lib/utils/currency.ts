/** Currency is stored as a single integer in the smallest unit (copper).
 *  Gold/silver/copper is purely a display format: 1g = 100s = 10,000c. */
const COPPER_PER_SILVER = 100
const COPPER_PER_GOLD = 10_000

export interface GoldParts {
  negative: boolean
  gold: number
  silver: number
  copper: number
}

export function splitGold(copper: number): GoldParts {
  const negative = copper < 0
  let remaining = Math.abs(Math.trunc(copper))

  const gold = Math.trunc(remaining / COPPER_PER_GOLD)
  remaining %= COPPER_PER_GOLD
  const silver = Math.trunc(remaining / COPPER_PER_SILVER)

  return { negative, gold, silver, copper: remaining % COPPER_PER_SILVER }
}

/** The one display rule: drop empty units, keep `0c` when there is nothing.
 *  `GoldAmount` colours these segments; `formatGold` joins them. */
export function goldSegments(copper: number): {
  negative: boolean
  segments: { unit: 'gold' | 'silver' | 'copper'; text: string }[]
} {
  const p = splitGold(copper)
  const segments: { unit: 'gold' | 'silver' | 'copper'; text: string }[] = []
  if (p.gold > 0) segments.push({ unit: 'gold', text: `${p.gold}g` })
  if (p.silver > 0) segments.push({ unit: 'silver', text: `${p.silver}s` })
  if (p.copper > 0 || segments.length === 0) {
    segments.push({ unit: 'copper', text: `${p.copper}c` })
  }
  return { negative: p.negative, segments }
}

/** `"1g 20s 30c"` — plain-text twin of GoldAmount, for input fields and other
 *  places that can't take markup. */
export function formatGold(copper: number): string {
  const { negative, segments } = goldSegments(copper)
  return (negative ? '-' : '') + segments.map((s) => s.text).join(' ')
}

/** Read what `formatGold` writes, plus what people actually type: `1g20s30c`,
 *  `2s`, spaces and case ignored. A bare number is copper, so the old
 *  copper-only habit keeps working. `null` on anything else. */
export function parseGold(text: string): number | null {
  const cleaned = text.trim().toLowerCase().replace(/\s+/g, '')
  if (cleaned === '') return null
  if (/^\d+$/.test(cleaned)) return Number(cleaned)

  const m = cleaned.match(/^(?:(\d+)g)?(?:(\d+)s)?(?:(\d+)c)?$/)
  if (!m) return null
  return (
    Number(m[1] ?? 0) * COPPER_PER_GOLD +
    Number(m[2] ?? 0) * COPPER_PER_SILVER +
    Number(m[3] ?? 0)
  )
}

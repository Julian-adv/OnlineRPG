/** Parse `/tp <x> <z> [y]` arguments; null when malformed. Values must stay
 *  finite after f32 rounding — the wire format is f32, so e.g. 1e39 would
 *  arrive server-side as Infinity. */
export function parseTpArgs(
  args: string
): { x: number; z: number; y: number } | null {
  const parts = args.trim().split(/\s+/).filter(Boolean)
  if (parts.length < 2 || parts.length > 3) return null
  const [x, z, y = 0] = parts.map(Number)
  if (![x, z, y].every((v) => Number.isFinite(Math.fround(v)))) return null
  return { x, z, y }
}

/** Resolve a single-token `/tp` argument against the destination list:
 *  a 1-based list index, or a case-insensitive destination name. */
export function resolveTpDestination<T extends { name: string }>(
  token: string,
  destinations: T[]
): T | null {
  const trimmed = token.trim()
  if (/^\d+$/.test(trimmed)) {
    return destinations[parseInt(trimmed, 10) - 1] ?? null
  }
  const lower = trimmed.toLowerCase()
  return destinations.find((d) => d.name.toLowerCase() === lower) ?? null
}

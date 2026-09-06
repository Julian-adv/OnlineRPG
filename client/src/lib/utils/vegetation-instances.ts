const FLOATS_PER_INSTANCE = 5

export function filterVegetationInstances(
  raw: Float32Array,
  shouldRemove: (x: number, z: number) => boolean
): Float32Array {
  const count = raw.length / FLOATS_PER_INSTANCE
  let kept = 0
  for (let i = 0; i < count; i++) {
    const base = i * FLOATS_PER_INSTANCE
    if (!shouldRemove(raw[base], raw[base + 2])) kept++
  }
  if (kept === count) return raw
  const out = new Float32Array(kept * FLOATS_PER_INSTANCE)
  let offset = 0
  for (let i = 0; i < count; i++) {
    const base = i * FLOATS_PER_INSTANCE
    if (shouldRemove(raw[base], raw[base + 2])) continue
    out.set(raw.subarray(base, base + FLOATS_PER_INSTANCE), offset)
    offset += FLOATS_PER_INSTANCE
  }
  return out
}

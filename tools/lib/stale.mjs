/**
 * Staleness check shared by the generators that write committed artifacts.
 *
 * A predicate rather than a skip-and-exit, so each caller keeps its own message
 * and control flow — `measure-furniture-footprints.mjs` in particular has to
 * decide before its lazy `@gltf-transform/core` import.
 */
import { statSync, existsSync } from 'fs'

/** True when `outPath` exists and is at least as new as every input. */
export function upToDate(outPath, inputs) {
  if (process.argv.includes('--force') || !existsSync(outPath)) return false
  const out = statSync(outPath).mtimeMs
  return inputs.filter(existsSync).every((p) => statSync(p).mtimeMs <= out)
}

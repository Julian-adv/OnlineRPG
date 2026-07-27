#!/usr/bin/env node
/**
 * Extract player animation durations from GLB files to
 * agent-client/data/animation_durations.json, which the agent-client reads at
 * runtime to pace its attacks off the real clip length instead of a guess.
 *
 * GLB parsing lives in lib/glb-animations.mjs, shared with
 * measure-monster-attack-clips.mjs.
 *
 * Runs as a step of `build:wasm`; it self-skips (a few stat() calls, no GLB
 * reads) when the output is already newer than every input.
 *
 *   node tools/extract-animation-durations.mjs           # regenerate if stale
 *   node tools/extract-animation-durations.mjs --force   # regenerate always
 */

import { writeFileSync, existsSync } from 'fs'
import { extractDurations } from './lib/glb-animations.mjs'
import { upToDate } from './lib/stale.mjs'
import { resolve, dirname } from 'path'
import { fileURLToPath } from 'url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..')

const GLB_FILES = [
  'client/public/models/animations/combat_melee.glb',
  'client/public/models/animations/locomotion.glb',
  'client/public/models/characters/female_knight.glb',
]

const OUT_PATH = resolve(ROOT, 'agent-client/data/animation_durations.json')
if (upToDate(OUT_PATH, GLB_FILES.map((p) => resolve(ROOT, p)))) {
  console.log(
    'animation durations up to date — skipping (use --force to regenerate)'
  )
  process.exit(0)
}

const allDurations = {}

for (const relPath of GLB_FILES) {
  const fullPath = resolve(ROOT, relPath)
  try {
    const durations = extractDurations(fullPath)
    const source = relPath.replace(/.*\//, '').replace('.glb', '')
    allDurations[source] = durations
    console.log(`✓ ${relPath}: ${Object.keys(durations).length} animations`)
  } catch (e) {
    console.warn(`⚠ Skipping ${relPath}: ${e.message}`)
  }
}

// All GLBs unreadable means an LFS-less checkout (CI) — keep the committed
// durations instead of clobbering them with an empty file.
if (Object.keys(allDurations).length === 0 && existsSync(OUT_PATH)) {
  console.warn('no readable GLBs — keeping committed animation durations')
  process.exit(0)
}

writeFileSync(OUT_PATH, JSON.stringify(allDurations, null, 2) + '\n')
console.log(`\nWrote ${OUT_PATH}`)

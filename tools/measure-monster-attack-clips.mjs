#!/usr/bin/env node
/**
 * Measure how long each monster's attack animation runs and write it to
 * data/monster_attack_clips.json — the single source of truth the shared
 * `monster_ai` crate embeds at compile time, so the browser (wasm) and the
 * headless agent-client hold a swing for exactly as long as the clip they play.
 *
 * Authoring source is data/monsters.json (`model` + `animAttack`), so a clip
 * re-exported at a different length needs a re-run rather than a hand-edited
 * number that can drift from the model.
 *
 *   node tools/measure-monster-attack-clips.mjs           # regenerate if stale
 *   node tools/measure-monster-attack-clips.mjs --force   # regenerate always
 */
import { readFileSync, writeFileSync, existsSync } from 'fs'
import { resolve, dirname } from 'path'
import { fileURLToPath } from 'url'
import { extractDurations } from './lib/glb-animations.mjs'
import { upToDate } from './lib/stale.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const MONSTERS_PATH = resolve(ROOT, 'data/monsters.json')
const MODELS_DIR = resolve(ROOT, 'client/public/models')
const OUT_PATH = resolve(ROOT, 'data/monster_attack_clips.json')

// monsters.json is untracked authoring data; a checkout without it (CI, fresh
// clone) builds from the committed output instead.
if (!existsSync(MONSTERS_PATH)) {
  if (existsSync(OUT_PATH)) {
    console.log('data/monsters.json missing — keeping committed attack clips')
    process.exit(0)
  }
  console.error(
    'data/monsters.json missing and no committed data/monster_attack_clips.json to fall back on'
  )
  process.exit(1)
}

const modelPath = (m) => resolve(MODELS_DIR, m.model)
const monsters = Object.values(
  JSON.parse(readFileSync(MONSTERS_PATH, 'utf8'))
).filter((m) => m.id && m.model)

if (upToDate(OUT_PATH, [MONSTERS_PATH, ...monsters.map(modelPath)])) {
  console.log(
    'monster attack clips up to date — skipping (use --force to regenerate)'
  )
  process.exit(0)
}

// Bosses reuse their base type's model, so measure each GLB once.
const durationsByModel = new Map()
const clips = {}

for (const m of monsters) {
  if (!existsSync(modelPath(m))) {
    console.warn(`⚠ ${m.id}: no model at ${m.model}`)
    continue
  }
  if (!durationsByModel.has(m.model)) {
    durationsByModel.set(m.model, extractDurations(modelPath(m)))
  }
  const clipName = m.animAttack ?? 'Attack'
  const seconds = durationsByModel.get(m.model)[clipName]
  if (seconds == null) {
    console.warn(`⚠ ${m.id}: ${m.model} has no clip "${clipName}"`)
    continue
  }
  clips[m.id] = Math.round(seconds * 1000)
}

const sorted = Object.fromEntries(
  Object.entries(clips).sort(([a], [b]) => a.localeCompare(b))
)
const text = JSON.stringify(sorted, null, 2) + '\n'

// Only write on a real change: the shared crate `include_str!`s this, so an
// identical rewrite would still cost a full recompile of everything above it.
if (!existsSync(OUT_PATH) || readFileSync(OUT_PATH, 'utf8') !== text) {
  writeFileSync(OUT_PATH, text)
  console.log(`Wrote ${Object.keys(sorted).length} attack clip lengths to ${OUT_PATH}`)
} else {
  console.log('monster attack clips unchanged')
}

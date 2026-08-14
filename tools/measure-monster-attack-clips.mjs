#!/usr/bin/env node
/**
 * Measure how long each monster's attack animation runs and write it to
 * data/monster_attack_clips.json — the single source of truth the shared
 * `monster_ai` crate embeds at compile time, so the browser (wasm) and the
 * headless agent-client hold a swing for exactly as long as the clip they play.
 *
 * Authoring source is data/monsters.json (generated from data-src/monsters.csv
 * by generate:csv; `model` + `animAttack`), so a clip
 * re-exported at a different length needs a re-run rather than a hand-edited
 * number that can drift from the model.
 *
 *   node tools/measure-monster-attack-clips.mjs           # regenerate if stale
 *   node tools/measure-monster-attack-clips.mjs --force   # regenerate always
 */
import {
  readFileSync,
  writeFileSync,
  existsSync,
  openSync,
  readSync,
  closeSync,
} from 'fs'
import { resolve, dirname } from 'path'
import { fileURLToPath } from 'url'
import { extractDurations } from './lib/glb-animations.mjs'
import { upToDate } from './lib/stale.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const MONSTERS_PATH = resolve(ROOT, 'data/monsters.json')
const MODELS_DIR = resolve(ROOT, 'client/public/models')
const OUT_PATH = resolve(ROOT, 'data/monster_attack_clips.json')

// monsters.json is generated from data-src/monsters.csv; build:wasm runs
// generate:csv first, so a missing file means that step was skipped.
if (!existsSync(MONSTERS_PATH)) {
  console.error('data/monsters.json missing — run `npm run generate:csv` first')
  process.exit(1)
}

// Monsters rigged on the character skeleton swing a clip from the shared packs,
// not one of their own, so those packs are measured alongside the models.
const SHARED_PACKS = ['animations/locomotion.glb', 'animations/combat_melee.glb']

const modelPath = (m) => resolve(MODELS_DIR, m.model)
const monsters = Object.values(
  JSON.parse(readFileSync(MONSTERS_PATH, 'utf8'))
).filter((m) => m.id && m.model)
const sharedPackPaths = monsters.some((m) => m.sharedAnims)
  ? SHARED_PACKS.map((p) => resolve(MODELS_DIR, p))
  : []

if (
  upToDate(OUT_PATH, [
    MONSTERS_PATH,
    ...monsters.map(modelPath),
    ...sharedPackPaths,
  ])
) {
  console.log(
    'monster attack clips up to date — skipping (use --force to regenerate)'
  )
  process.exit(0)
}

const isGlb = (p) => {
  const buf = Buffer.alloc(4)
  const fd = openSync(p, 'r')
  readSync(fd, buf, 0, 4, 0)
  closeSync(fd)
  return buf.readUInt32LE(0) === 0x46546c67
}

// Any model we can't measure means an incomplete result, so bail rather than
// clobber the committed clips: a checkout without LFS content (CI does a plain
// checkout to stay off the LFS bandwidth quota) has pointer text files instead
// of GLBs, and one that never ran fetch-assets.sh has no monster models at all.
// Mirrors how measure-furniture-footprints.mjs handles missing tool deps.
const models = [...new Set([...monsters.map(modelPath), ...sharedPackPaths])]
if (!models.every((p) => existsSync(p) && isGlb(p))) {
  if (existsSync(OUT_PATH)) {
    console.warn(
      'monster models missing or git-lfs pointers — keeping committed attack clips'
    )
    process.exit(0)
  }
  console.error(
    'monster models missing or git-lfs pointers, and no committed data/monster_attack_clips.json to fall back on'
  )
  process.exit(1)
}

// Bosses reuse their base type's model, so measure each GLB once.
const durationsByModel = new Map()
const clips = {}

const sharedDurations = Object.assign(
  {},
  ...sharedPackPaths.map((p) => extractDurations(p))
)

for (const m of monsters) {
  if (!existsSync(modelPath(m))) {
    console.warn(`⚠ ${m.id}: no model at ${m.model}`)
    continue
  }
  if (!durationsByModel.has(m.model)) {
    durationsByModel.set(m.model, extractDurations(modelPath(m)))
  }
  const clipName = m.animAttack ?? 'Attack'
  const seconds =
    durationsByModel.get(m.model)[clipName] ??
    (m.sharedAnims ? sharedDurations[clipName] : undefined)
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

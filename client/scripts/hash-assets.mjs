// Adds content-hashed copies (name.<sha1:8>.ext) of the runtime assets in dist
// and inlines an original -> hashed path map into dist/index.html. Originals
// stay in place for old bundles and external references.
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

const dist = path.resolve(import.meta.dirname, '../dist')
const DIRS = ['textures', 'models', 'bgm', 'sounds', 'portraits']
const EXTS = /\.(glb|png|jpe?g|webp|mp3|m4a|ogg)$/i
const HASHED = /\.[0-9a-f]{8}\.[^.]+$/

const manifest = {}

function walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      walk(full)
      continue
    }
    if (!EXTS.test(entry.name) || HASHED.test(entry.name)) continue
    const hash = createHash('sha1')
      .update(fs.readFileSync(full))
      .digest('hex')
      .slice(0, 8)
    const ext = path.extname(entry.name)
    const hashedFull = path.join(
      dir,
      `${entry.name.slice(0, -ext.length)}.${hash}${ext}`
    )
    if (!fs.existsSync(hashedFull)) {
      try {
        fs.linkSync(full, hashedFull)
      } catch {
        fs.copyFileSync(full, hashedFull)
      }
    }
    const rel = (p) => '/' + path.relative(dist, p).split(path.sep).join('/')
    manifest[rel(full)] = rel(hashedFull)
  }
}

for (const d of DIRS) {
  const dir = path.join(dist, d)
  if (fs.existsSync(dir)) walk(dir)
}

const indexPath = path.join(dist, 'index.html')
const tag = `<script>window.__ASSET_MANIFEST__=${JSON.stringify(manifest).replaceAll('</', '<\\/')}</script>`
fs.writeFileSync(
  indexPath,
  fs.readFileSync(indexPath, 'utf8').replace('<head>', `<head>${tag}`)
)
console.log(`hash-assets: ${Object.keys(manifest).length} assets`)

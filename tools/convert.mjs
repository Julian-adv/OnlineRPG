import fs from 'fs'
import path from 'path'
import { fileURLToPath } from 'url'

const toolsDir = path.dirname(fileURLToPath(import.meta.url))
const sourceDir = path.join(toolsDir, '..', 'data-src')
const dataDir = path.join(toolsDir, '..', 'data')

/**
 * Convert a single CSV file to a JSON file keyed by the `id` column.
 * Returns the number of entries converted.
 */
export function convertCsvFile(csvFileName) {
  const csvPath = path.join(sourceDir, csvFileName)
  const jsonFileName = csvFileName.replace(/\.csv$/, '.json')
  const jsonPath = path.join(dataDir, jsonFileName)

  fs.mkdirSync(dataDir, { recursive: true })
  const csv = fs.readFileSync(csvPath, 'utf-8').trim()
  const lines = csv.split('\n')
  const headers = lines[0].split(',')

  const entries = {}

  for (let i = 1; i < lines.length; i++) {
    const line = lines[i].trim()
    if (!line) continue

    const values = line.split(',')
    const entry = {}

    for (let j = 0; j < headers.length; j++) {
      const key = headers[j].trim()
      // Tolerate short rows. The Rust build tool rejects them instead, so a
      // field-count mismatch fails the cargo build rather than this one.
      const raw = (values[j] ?? '').trim()

      // Parse booleans
      if (raw === 'true' || raw === 'false') {
        entry[key] = raw === 'true'
        continue
      }

      // Skip empty values
      if (raw === '') continue

      // Try to parse as number, keep as string otherwise
      const num = Number(raw)
      entry[key] = Number.isNaN(num) ? raw : num
    }

    entries[entry.id] = entry
  }

  fs.writeFileSync(jsonPath, JSON.stringify(entries, null, 2) + '\n')
  return { count: Object.keys(entries).length, jsonPath }
}

// Run directly from CLI
const isDirectRun =
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))

if (isDirectRun) {
  const csvFiles = fs.readdirSync(sourceDir).filter((f) => f.endsWith('.csv'))
  for (const csvFile of csvFiles) {
    const { count, jsonPath } = convertCsvFile(csvFile)
    console.log(`Converted ${count} entry/entries from ${csvFile} -> ${jsonPath}`)
  }
}

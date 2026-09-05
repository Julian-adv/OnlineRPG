import { writable } from 'svelte/store'
import { apiFetch, getTerrainApiUrl } from '../utils/networkUtils'
import { REGION_PLOTS } from '../terrain/landPlots'
import { regionKey } from '../terrain/terrain-constants'
import { wrapRegionX } from '../terrain/world-wrap'

const FAILED_RETRY_MS = 5000

const cache = new Map<string, Uint8Array>()
const inflight = new Set<string>()
const failedAt = new Map<string, number>()

/** Bumped when a region loads, is edited, or is evicted so map renders re-run. */
export const landGradeVersion = writable(0)

function bump() {
  landGradeVersion.update((v) => v + 1)
}

function key(rx: number, rz: number) {
  return regionKey(wrapRegionX(rx), rz)
}

function url(rx: number, rz: number) {
  return `${getTerrainApiUrl()}/api/terrain/land-grades/${wrapRegionX(rx)}/${rz}`
}

export function getCachedLandGrades(rx: number, rz: number) {
  return cache.get(key(rx, rz)) ?? null
}

/** Starts a fetch for a region not in the cache; the version store signals
 *  arrival. Failures back off so a dragging map does not hammer the server. */
export function requestLandGrades(rx: number, rz: number): void {
  const k = key(rx, rz)
  if (cache.has(k) || inflight.has(k)) return
  const failed = failedAt.get(k)
  if (failed !== undefined && Date.now() - failed < FAILED_RETRY_MS) return
  inflight.add(k)
  fetch(url(rx, rz))
    .then(async (resp) => {
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`)
      const bytes = new Uint8Array(await resp.arrayBuffer())
      if (bytes.length !== REGION_PLOTS) throw new Error('bad length')
      cache.set(k, bytes)
      failedAt.delete(k)
      bump()
    })
    .catch(() => failedAt.set(k, Date.now()))
    .finally(() => inflight.delete(k))
}

/** Edits the cached byte immediately and writes the whole region back. On
 *  failure the region is evicted so the next render shows the server's truth. */
export async function setLandGrade(
  rx: number,
  rz: number,
  index: number,
  grade: number
): Promise<void> {
  const k = key(rx, rz)
  const grades = cache.get(k)
  if (!grades) return
  grades[index] = grade
  bump()
  const resp = await apiFetch(url(rx, rz), {
    method: 'PUT',
    headers: { 'Content-Type': 'application/octet-stream' },
    body: grades,
  }).catch(() => null)
  if (!resp?.ok) {
    cache.delete(k)
    bump()
    throw new Error(resp ? `HTTP ${resp.status}` : 'network error')
  }
}

import { regionMinimapServerUrl } from './regionMinimapGenerator'
import { regionKey } from './terrain-constants'
import { wrapRegionX } from './world-wrap'

/** Failed loads back off from here (doubling, capped) so a transient error
 *  doesn't blank a region for good while missing regions aren't hammered. */
const ERROR_RETRY_MS = 10_000
const ERROR_RETRY_MAX_MS = 300_000

/** Loader/cache for baked region minimap images, shared by the world map and
 *  the HUD minimap. Wraps rx at the world seam; auto-flushes when the bake
 *  version moves, and version-guards in-flight loads against stale writes. */
export class RegionImageCache {
  private images = new Map<string, HTMLImageElement>()
  private pending = new Map<string, Promise<HTMLImageElement | null>>()
  private failed = new Map<string, { retryAt: number; backoff: number }>()
  private version: number | null = null
  private maxImages = Infinity

  get limit(): number {
    return this.maxImages
  }

  set limit(value: number) {
    this.maxImages = value
    this.trim()
  }

  load(
    rx: number,
    rz: number,
    version: number
  ): Promise<HTMLImageElement | null> {
    if (version !== this.version) {
      this.flush()
      this.version = version
    }

    const wrx = wrapRegionX(rx)
    const key = regionKey(wrx, rz)
    const cached = this.images.get(key)
    if (cached) return Promise.resolve(cached)
    const failure = this.failed.get(key)
    if (failure && Date.now() < failure.retryAt) return Promise.resolve(null)
    const inFlight = this.pending.get(key)
    if (inFlight) return inFlight

    const promise = new Promise<HTMLImageElement | null>((resolve) => {
      const img = new Image()
      img.onload = () => {
        if (version === this.version) {
          this.images.set(key, img)
          this.failed.delete(key)
          this.pending.delete(key)
          this.trim()
        }
        resolve(img)
      }
      img.onerror = () => {
        if (version === this.version) {
          const prev = this.failed.get(key)
          const backoff = prev
            ? Math.min(prev.backoff * 2, ERROR_RETRY_MAX_MS)
            : ERROR_RETRY_MS
          this.failed.set(key, { retryAt: Date.now() + backoff, backoff })
          this.pending.delete(key)
        }
        resolve(null)
      }
      img.src = regionMinimapServerUrl(wrx, rz, version)
    })
    this.pending.set(key, promise)
    return promise
  }

  flush() {
    this.images.clear()
    this.pending.clear()
    this.failed.clear()
    this.version = null
  }

  private trim() {
    if (!Number.isFinite(this.maxImages) || this.images.size <= this.maxImages)
      return
    for (const key of this.images.keys()) {
      this.images.delete(key)
      if (this.images.size <= this.maxImages) break
    }
  }
}

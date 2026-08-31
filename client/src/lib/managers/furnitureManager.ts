import type { ObjectPlacement } from '../stores/editorStore'
import { isNeighbourRegion } from '../terrain/terrain-constants'
import {
  passability_set_furniture,
  passability_remove_furniture,
  furniture_is_solid,
} from '../wasm/onlinerpg_shared'

/** One sealed furniture piece, as returned by the wasm setter for the debug
 *  overlay: the world grid cells it occupies, the floor Y it sits on, and
 *  its floor level (the overlay draws only the player's floor). */
export interface FurnitureDebugPiece {
  cells: [number, number][]
  yBase: number
  floorLevel: number
}

/**
 * Forwards editor-placed region objects to the shared wasm passability cache so
 * solid furniture blocks movement AND click-to-move A* (a character can neither
 * walk through it nor path through it).
 *
 * All collision logic — which types are solid, their footprint, the sealed
 * cells — lives in shared Rust (`furniture` module), so the browser, the
 * agent-client (bots) and the server all agree on the exact same cells. This
 * manager just hands the raw placements to wasm and keeps the returned debug
 * pieces for the passability overlay.
 */
class FurnitureManager {
  /** Regions currently held in the wasm passability cache, with their sealed
   *  pieces for the debug overlay. */
  private regions = new Map<
    string,
    { rx: number; rz: number; pieces: FurnitureDebugPiece[] }
  >()
  private changeListeners: (() => void)[] = []

  private cacheKey(rx: number, rz: number): string {
    return `furniture:${rx},${rz}`
  }

  /** Whether an object type is solid furniture (blocks movement). Authoritative
   *  in shared Rust (`furniture::is_solid`). */
  isSolid(type: string): boolean {
    return furniture_is_solid(type)
  }

  /** Solid furniture is constrained to 90° yaw so its footprint lands on whole
   *  cells; other objects keep their free rotation. */
  snapRotation(type: string, rotationDeg: number): number {
    if (!this.isSolid(type)) return rotationDeg
    return (((Math.round(rotationDeg / 90) * 90) % 360) + 360) % 360
  }

  /** Re-register a region's furniture from its raw object placements. wasm
   *  resolves solidity + footprints and returns the sealed pieces for debug. */
  syncRegion(rx: number, rz: number, placements: ObjectPlacement[]): void {
    const key = this.cacheKey(rx, rz)
    let debug: FurnitureDebugPiece[] | undefined
    try {
      debug = passability_set_furniture(key, placements) as
        | FurnitureDebugPiece[]
        | undefined
    } catch (err) {
      // A single malformed placement would otherwise throw out of the Svelte
      // store subscription that calls this, breaking the whole reactive update.
      // The wasm setter validates before touching the cache, so the prior
      // entry (if any) is left intact — skip this sync and keep going.
      console.error(`[furniture] failed to sync region ${key}:`, err)
      return
    }
    if (debug && debug.length > 0) {
      this.regions.set(key, { rx, rz, pieces: debug })
    } else {
      // No solid furniture: the wasm setter removed the entry rather than
      // inserting an empty one, so there is nothing left to track either.
      this.regions.delete(key)
    }
    this.notifyChanged()
  }

  /** Drop cached regions more than one region away from (rx, rz), so the cache
   *  stops growing by one entry per region visited. The 8 neighbours are kept
   *  so furniture just across a boundary still collides — otherwise predicted
   *  movement would disagree with the server at region seams. */
  evictDistant(rx: number, rz: number): void {
    let removed = false
    for (const [key, region] of this.regions) {
      if (isNeighbourRegion(region.rx, region.rz, rx, rz)) continue
      passability_remove_furniture(key)
      this.regions.delete(key)
      removed = true
    }
    if (removed) this.notifyChanged()
  }

  /** All sealed furniture pieces across loaded regions, for the debug overlay. */
  getDebugPieces(): FurnitureDebugPiece[] {
    const all: FurnitureDebugPiece[] = []
    for (const region of this.regions.values()) all.push(...region.pieces)
    return all
  }

  /** Subscribe to furniture changes (for the debug overlay to mark dirty).
   *  Returns an unsubscribe function. */
  onChanged(cb: () => void): () => void {
    this.changeListeners.push(cb)
    return () => {
      this.changeListeners = this.changeListeners.filter((l) => l !== cb)
    }
  }

  private notifyChanged(): void {
    for (const cb of this.changeListeners) cb()
  }
}

export const furnitureManager = new FurnitureManager()

import { SvelteMap } from 'svelte/reactivity'
import { hmrSingleton } from '../utils/hmr'
import type { Position, ServerCampfire } from '../network/networkTypes'

/** Burning campfires by id. Surface-only, so no floor bookkeeping. */
class CampfireManager {
  fires = new SvelteMap<number, Position>()

  spawn(campfire: ServerCampfire) {
    this.fires.set(campfire.id, { ...campfire.position })
  }

  remove(id: number) {
    this.fires.delete(id)
  }

  reset() {
    this.fires.clear()
  }
}

export const campfireManager = hmrSingleton(
  'campfireManager',
  () => new CampfireManager()
)

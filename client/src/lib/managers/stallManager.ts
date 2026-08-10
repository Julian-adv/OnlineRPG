import { SvelteMap } from 'svelte/reactivity'
import { hmrSingleton } from '../utils/hmr'
import type { ServerStall } from '../network/networkTypes'

/** Laid-out merchant stalls by id. Surface-only, like campfires. */
class StallManager {
  stalls = new SvelteMap<number, ServerStall>()

  spawn(stall: ServerStall) {
    this.stalls.set(stall.id, { ...stall })
  }

  remove(id: number) {
    this.stalls.delete(id)
  }

  reset() {
    this.stalls.clear()
  }
}

export const stallManager = hmrSingleton(
  'stallManager',
  () => new StallManager()
)

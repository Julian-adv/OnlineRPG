import { SvelteMap } from 'svelte/reactivity'
import { hmrSingleton } from '../utils/hmr'
import type { ServerTipHat } from '../network/networkTypes'

/** Standing tip hats by id. Surface-only, like campfires and stalls. */
class TipHatManager {
  hats = new SvelteMap<number, ServerTipHat>()

  spawn(hat: ServerTipHat) {
    this.hats.set(hat.id, { ...hat })
  }

  remove(id: number) {
    this.hats.delete(id)
  }

  reset() {
    this.hats.clear()
  }
}

export const tipHatManager = hmrSingleton(
  'tipHatManager',
  () => new TipHatManager()
)

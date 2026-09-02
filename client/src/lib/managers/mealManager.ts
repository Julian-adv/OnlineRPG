import { SvelteMap } from 'svelte/reactivity'
import { hmrSingleton } from '../utils/hmr'
import { clearNameHover } from '../stores/gameStore'
import type { ServerMeal } from '../network/networkTypes'

/** Served table plates by id. Surface-only, like tip hats and stalls. */
class MealManager {
  meals = new SvelteMap<number, ServerMeal>()

  spawn(meal: ServerMeal) {
    this.meals.set(meal.id, { ...meal })
  }

  markEaten(id: number) {
    const meal = this.meals.get(id)
    if (meal) this.meals.set(id, { ...meal, eaten: true })
  }

  remove(id: number) {
    clearNameHover()
    this.meals.delete(id)
  }

  reset() {
    this.meals.clear()
  }
}

export const mealManager = hmrSingleton('mealManager', () => new MealManager())

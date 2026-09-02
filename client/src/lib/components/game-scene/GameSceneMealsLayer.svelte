<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { SvelteMap, SvelteSet } from 'svelte/reactivity'
  import { mealManager } from '../../managers/mealManager'
  import { currentDungeonDepth } from '../../stores/dungeonStore'
  import { getItemDef } from '../../data/itemDefs'
  import { loadGLB } from '../../utils/gltfCache'
  import { hoverMetrics, type HoverMetrics } from '../../utils/hoverMetrics'
  import type { ServerMeal } from '../../network/networkTypes'

  type DishModel = { model: THREE.Group; hover: HoverMetrics }

  let group = $state<THREE.Group | undefined>(undefined)
  // One loaded model per GLB path, fetched the first time a plate needs it.
  const dishes = new SvelteMap<string, DishModel>()
  const loading = new SvelteSet<string>()

  /** A finished dish turns into the same empty plate whatever it held; a
   *  drink keeps its cup. */
  function mealLook(meal: ServerMeal): {
    path: string | undefined
    hoverName: string
  } {
    const def = getItemDef(meal.item_def_id)
    const name = def?.name ?? meal.item_def_id
    const own = def?.worldModel && `/models/${def.worldModel}`
    if (!meal.eaten) return { path: own, hoverName: name }
    return def?.category === 'drink'
      ? { path: own, hoverName: `Empty ${name} cup` }
      : { path: '/models/objects/empty_plate.glb', hoverName: 'Empty plate' }
  }

  // Meals stand on inn tables only; hide them while underground.
  const mealEntries = $derived(
    $currentDungeonDepth >= 1 ? [] : [...mealManager.meals]
  )

  $effect(() => {
    for (const [, meal] of mealEntries) {
      const { path } = mealLook(meal)
      if (path) ensureDish(path)
    }
  })

  function ensureDish(path: string) {
    if (dishes.has(path) || loading.has(path)) return
    loading.add(path)
    loadGLB(path)
      .then((gltf) => {
        gltf.scene.traverse((child) => {
          if (child instanceof THREE.Mesh) {
            child.castShadow = true
            child.receiveShadow = true
          }
        })
        dishes.set(path, {
          model: gltf.scene,
          hover: hoverMetrics(gltf.scene),
        })
      })
      .catch((error) => console.error('Failed to load meal model:', error))
      .finally(() => loading.delete(path))
  }

  export function getGroup(): THREE.Group | undefined {
    return group
  }
</script>

<T.Group bind:ref={group}>
  {#each mealEntries as [id, meal] (id)}
    {@const look = mealLook(meal)}
    {@const dish = dishes.get(look.path ?? '')}
    {#if dish}
      <T.Group
        position={[meal.position.x, meal.position.y, meal.position.z]}
        rotation={[0, meal.rotation, 0]}
        userData={{
          mealId: id,
          hoverName: look.hoverName,
          hoverLabelY: dish.hover.topY,
          hoverRingRadius: dish.hover.ringRadius,
          hoverCenter: { x: dish.hover.center.x, y: 0, z: dish.hover.center.z },
          hoverFloorLevel: meal.floor_level,
          hoverDrape: 0,
        }}
      >
        <T is={dish.model.clone(true)} />
      </T.Group>
    {/if}
  {/each}
</T.Group>

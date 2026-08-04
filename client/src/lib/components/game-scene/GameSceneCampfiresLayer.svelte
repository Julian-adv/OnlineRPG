<script lang="ts">
  import { T } from '@threlte/core'
  import { onDestroy, onMount } from 'svelte'
  import * as THREE from 'three'
  import { campfireManager } from '../../managers/campfireManager'
  import { currentDungeonDepth } from '../../stores/dungeonStore'
  import { CampfireFireParticles } from '../../effects/fire-particles'
  import { loadGLB } from '../../utils/gltfCache'

  let group = $state<THREE.Group | undefined>(undefined)
  let campfireModel = $state<THREE.Group | null>(null)
  // Imperative only (frame loop + diffing) — never read from the template.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const fireSystems = new Map<number, CampfireFireParticles>()

  // Campfires exist on the surface only; hide them while underground.
  const fireEntries = $derived(
    $currentDungeonDepth >= 1 ? [] : [...campfireManager.fires]
  )

  $effect(() => {
    if (!group) return
    for (const [id, system] of fireSystems) {
      if (!fireEntries.some(([fid]) => fid === id)) {
        system.dispose()
        fireSystems.delete(id)
      }
    }
    for (const [id, position] of fireEntries) {
      if (!fireSystems.has(id)) {
        const system = new CampfireFireParticles()
        system.setOrigin(
          new THREE.Vector3(position.x, position.y + 0.125, position.z)
        )
        group.add(system.group)
        fireSystems.set(id, system)
      }
    }
  })

  onMount(() => {
    let cancelled = false
    loadGLB('/models/objects/campfire.glb')
      .then((gltf) => {
        if (cancelled) return
        gltf.scene.traverse((child) => {
          if (child instanceof THREE.Mesh) {
            child.castShadow = true
            child.receiveShadow = true
          }
        })
        campfireModel = gltf.scene
      })
      .catch((error) => console.error('Failed to load campfire model:', error))
    return () => {
      cancelled = true
    }
  })

  onDestroy(() => {
    for (const system of fireSystems.values()) system.dispose()
    fireSystems.clear()
  })

  export function update(deltaTime: number, camera: THREE.Camera | undefined) {
    for (const system of fireSystems.values()) system.update(deltaTime, camera)
  }
</script>

<T.Group bind:ref={group}>
  {#if campfireModel}
    {#each fireEntries as [id, position] (id)}
      <T.Group position={[position.x, position.y, position.z]}>
        <T is={campfireModel.clone(true)} />
      </T.Group>
    {/each}
  {/if}
</T.Group>

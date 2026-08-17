<script lang="ts">
  import { T } from '@threlte/core'
  import { onMount } from 'svelte'
  import * as THREE from 'three'
  import { stallManager } from '../../managers/stallManager'
  import { currentDungeonDepth } from '../../stores/dungeonStore'
  import { loadGLB } from '../../utils/gltfCache'

  let stallModel = $state<THREE.Group | null>(null)
  let group = $state<THREE.Group | undefined>(undefined)

  // Stalls exist on the surface only; hide them while underground.
  const stallEntries = $derived(
    $currentDungeonDepth >= 1 ? [] : [...stallManager.stalls]
  )

  onMount(() => {
    let cancelled = false
    loadGLB('/models/objects/black_market_table.glb')
      .then((gltf) => {
        if (cancelled) return
        gltf.scene.traverse((child) => {
          if (child instanceof THREE.Mesh) {
            child.castShadow = true
            child.receiveShadow = true
          }
        })
        stallModel = gltf.scene
      })
      .catch((error) => console.error('Failed to load stall model:', error))
    return () => {
      cancelled = true
    }
  })

  export function getGroup(): THREE.Group | undefined {
    return group
  }
</script>

<T.Group bind:ref={group}>
  {#if stallModel}
    {#each stallEntries as [id, stall] (id)}
      <T.Group
        position={[stall.position.x, stall.position.y, stall.position.z]}
        rotation={[0, stall.rotation, 0]}
        userData={{ stallId: id }}
      >
        <T is={stallModel.clone(true)} />
      </T.Group>
    {/each}
  {/if}
</T.Group>

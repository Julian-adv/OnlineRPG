<script lang="ts">
  import { T } from '@threlte/core'
  import { onMount } from 'svelte'
  import * as THREE from 'three'
  import { tipHatManager } from '../../managers/tipHatManager'
  import { currentDungeonDepth } from '../../stores/dungeonStore'
  import { loadGLB } from '../../utils/gltfCache'
  import { hoverMetrics, type HoverMetrics } from '../../utils/hoverMetrics'

  let group = $state<THREE.Group | undefined>(undefined)
  let hatModel = $state<THREE.Group | null>(null)
  let hatHover = $state<HoverMetrics | null>(null)

  // Tip hats exist on the surface only; hide them while underground.
  const hatEntries = $derived(
    $currentDungeonDepth >= 1 ? [] : [...tipHatManager.hats]
  )

  onMount(() => {
    let cancelled = false
    loadGLB('/models/objects/tip_hat.glb')
      .then((gltf) => {
        if (cancelled) return
        gltf.scene.traverse((child) => {
          if (child instanceof THREE.Mesh) {
            child.castShadow = true
            child.receiveShadow = true
          }
        })
        hatModel = gltf.scene
        hatHover = hoverMetrics(gltf.scene)
      })
      .catch((error) => console.error('Failed to load tip hat model:', error))
    return () => {
      cancelled = true
    }
  })

  export function getGroup(): THREE.Group | undefined {
    return group
  }
</script>

<T.Group bind:ref={group}>
  {#if hatModel && hatHover}
    {#each hatEntries as [id, hat] (id)}
      <T.Group
        position={[hat.position.x, hat.position.y, hat.position.z]}
        rotation={[0, hat.rotation, 0]}
        userData={{
          tipHatId: id,
          hoverName: `${hat.owner_name}'s Tip Hat`,
          hoverLabelY: hatHover.topY,
          hoverRingRadius: hatHover.ringRadius,
          hoverCenter: { x: hatHover.center.x, y: 0, z: hatHover.center.z },
          hoverFloorLevel: hat.floor_level,
        }}
      >
        <T is={hatModel.clone(true)} />
      </T.Group>
    {/each}
  {/if}
</T.Group>

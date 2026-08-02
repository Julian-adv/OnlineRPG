<script lang="ts">
  import { T } from '@threlte/core'
  import { onDestroy } from 'svelte'
  import * as THREE from 'three'
  import { campfireManager } from '../../managers/campfireManager'
  import { currentDungeonDepth } from '../../stores/dungeonStore'
  import { CampfireFireParticles } from '../../effects/fire-particles'

  let group = $state<THREE.Group | undefined>(undefined)
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
          new THREE.Vector3(position.x, position.y + 0.25, position.z)
        )
        group.add(system.group)
        fireSystems.set(id, system)
      }
    }
  })

  onDestroy(() => {
    for (const system of fireSystems.values()) system.dispose()
    fireSystems.clear()
  })

  export function update(deltaTime: number, camera: THREE.Camera | undefined) {
    for (const system of fireSystems.values()) system.update(deltaTime, camera)
  }

  const LOG_ANGLES = [0, Math.PI / 3, (2 * Math.PI) / 3]
  const STONE_COUNT = 7
</script>

<T.Group bind:ref={group}>
  {#each fireEntries as [id, position] (id)}
    <T.Group position={[position.x, position.y, position.z]}>
      {#each LOG_ANGLES as angle (angle)}
        <T.Mesh
          rotation={[Math.PI / 2, 0, angle]}
          position={[0, 0.09, 0]}
          castShadow
        >
          <T.CylinderGeometry args={[0.05, 0.06, 0.85, 6]} />
          <T.MeshStandardMaterial color="#4a2f1b" roughness={0.9} />
        </T.Mesh>
      {/each}
      {#each Array.from({ length: STONE_COUNT }, (_, i) => i) as i (i)}
        <T.Mesh
          position={[
            Math.cos((i / STONE_COUNT) * Math.PI * 2) * 0.55,
            0.05,
            Math.sin((i / STONE_COUNT) * Math.PI * 2) * 0.55,
          ]}
        >
          <T.SphereGeometry args={[0.09, 6, 5]} />
          <T.MeshStandardMaterial color="#6e6a63" roughness={1} />
        </T.Mesh>
      {/each}
      <T.Mesh position={[0, 0.12, 0]}>
        <T.SphereGeometry args={[0.16, 8, 6]} />
        <T.MeshStandardMaterial
          color="#ff7722"
          emissive="#ff5500"
          emissiveIntensity={2}
        />
      </T.Mesh>
    </T.Group>
  {/each}
</T.Group>

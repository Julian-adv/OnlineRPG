<script lang="ts" module>
  import * as THREE from 'three'

  const SEGMENTS = 48
  const LIFT = 0.06

  // Constant across every ring instance: the triangle index (shared as one
  // BufferAttribute) and the unit-circle table the drape loop walks.
  const indices: number[] = []
  for (let i = 0; i < SEGMENTS; i++) {
    const a = i * 2
    const b = a + 1
    const c = ((i + 1) % SEGMENTS) * 2
    const d = c + 1
    indices.push(a, c, b, b, c, d)
  }
  const RING_INDEX = new THREE.BufferAttribute(new Uint16Array(indices), 1)
  const UNIT_COS = new Float32Array(SEGMENTS)
  const UNIT_SIN = new Float32Array(SEGMENTS)
  for (let i = 0; i < SEGMENTS; i++) {
    const angle = (i / SEGMENTS) * Math.PI * 2
    UNIT_COS[i] = Math.cos(angle)
    UNIT_SIN[i] = Math.sin(angle)
  }
</script>

<script lang="ts">
  import { T } from '@threlte/core'
  import { onDestroy } from 'svelte'
  import { entityGroundY } from '../managers/entity-ground'
  import type { TerrainHeightManager } from '../managers/terrainHeightManager'

  interface Props {
    heightManager: TerrainHeightManager | null
    x: number
    z: number
    radius: number
    floorLevel?: number
    fallbackY?: number
    color?: string
  }

  let {
    heightManager,
    x,
    z,
    radius,
    floorLevel = 0,
    fallbackY = 0,
    color = '#ff4d4d',
  }: Props = $props()

  const geometry = new THREE.BufferGeometry()
  const positions = new Float32Array(SEGMENTS * 2 * 3)
  geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))
  geometry.setIndex(RING_INDEX)
  onDestroy(() => geometry.dispose())

  // Drape each vertex on the ground so the ring follows slopes like a decal.
  // Heights are clamped around the entity's own y: at a stair or cliff edge
  // the sampled ground can drop meters below, and unclamped vertices stretch
  // the band into near-vertical strips.
  $effect(() => {
    const width = Math.max(0.08, radius * 0.12)
    const inner = Math.max(0.05, radius - width)
    const maxStep = Math.max(0.4, radius * 0.6)
    const attr = geometry.getAttribute('position')
    for (let i = 0; i < SEGMENTS; i++) {
      const cos = UNIT_COS[i]
      const sin = UNIT_SIN[i]
      for (let slot = 0; slot < 2; slot++) {
        const r = slot === 0 ? inner : radius
        const vx = x + cos * r
        const vz = z + sin * r
        const groundY = entityGroundY(
          heightManager,
          floorLevel,
          vx,
          vz,
          fallbackY,
          fallbackY
        )
        const vy =
          Math.min(
            Math.max(groundY, fallbackY - maxStep),
            fallbackY + maxStep
          ) + LIFT
        attr.setXYZ(i * 2 + slot, vx, vy, vz)
      }
    }
    attr.needsUpdate = true
  })
</script>

<T.Mesh {geometry} frustumCulled={false} renderOrder={1}>
  <T.MeshBasicMaterial
    {color}
    transparent
    opacity={0.85}
    depthWrite={false}
    side={THREE.DoubleSide}
    polygonOffset
    polygonOffsetFactor={-4}
    toneMapped={false}
  />
</T.Mesh>

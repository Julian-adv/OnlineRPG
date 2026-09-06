<script lang="ts">
  import * as THREE from 'three'
  import { T } from '@threlte/core'
  import { onDestroy } from 'svelte'
  import {
    editorTool,
    roadDrawStart,
    editorHeightManager,
    brushWorldPos,
    brushSize,
  } from '../../stores/editorStore'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'

  const PREVIEW_COLOR = new THREE.Color(0xffcc00)
  const Y_OFFSET = 0.15

  // Persistent geometries, updated in place to avoid per-frame GC churn.
  const fillGeo = new THREE.BufferGeometry()
  const borderGeo = new THREE.BufferGeometry()
  let fillPositions: Float32Array | null = null
  let fillIndices: Uint32Array | null = null
  let borderPositions: Float32Array | null = null
  let cachedSegments = -1

  function updateStrip(
    mgr: TerrainHeightManager,
    x1: number,
    z1: number,
    x2: number,
    z2: number,
    r: number
  ): boolean {
    const dx = x2 - x1
    const dz = z2 - z1
    const len = Math.sqrt(dx * dx + dz * dz)
    if (len < 0.5) return false

    const ux = dx / len
    const uz = dz / len
    // Left-hand perpendicular
    const px = -uz
    const pz = ux

    const segments = Math.max(1, Math.ceil(len))
    const vertCount = (segments + 1) * 2

    if (segments !== cachedSegments) {
      fillPositions = new Float32Array(vertCount * 3)
      borderPositions = new Float32Array(vertCount * 3)
      fillIndices = new Uint32Array(segments * 6)
      for (let i = 0, ii = 0; i < segments; i++) {
        const a = i * 2
        fillIndices[ii++] = a
        fillIndices[ii++] = a + 2
        fillIndices[ii++] = a + 1
        fillIndices[ii++] = a + 1
        fillIndices[ii++] = a + 2
        fillIndices[ii++] = a + 3
      }
      fillGeo.setAttribute(
        'position',
        new THREE.BufferAttribute(fillPositions, 3)
      )
      fillGeo.setIndex(new THREE.BufferAttribute(fillIndices, 1))
      borderGeo.setAttribute(
        'position',
        new THREE.BufferAttribute(borderPositions, 3)
      )
      cachedSegments = segments
    }

    const fp = fillPositions!
    const bp = borderPositions!

    for (let i = 0; i <= segments; i++) {
      const t = i / segments
      const cx = x1 + dx * t
      const cz = z1 + dz * t

      const lx = cx + px * r
      const lz = cz + pz * r
      const rx = cx - px * r
      const rz = cz - pz * r

      const ly = mgr.getHeightAtWorldPosition(lx, lz) + Y_OFFSET
      const ry = mgr.getHeightAtWorldPosition(rx, rz) + Y_OFFSET

      const base = i * 2 * 3
      fp[base] = lx
      fp[base + 1] = ly
      fp[base + 2] = lz
      fp[base + 3] = rx
      fp[base + 4] = ry
      fp[base + 5] = rz
    }

    // Border: left edge forward, right edge back
    for (let i = 0; i <= segments; i++) {
      const src = i * 2 * 3
      const dst = i * 3
      bp[dst] = fp[src]
      bp[dst + 1] = fp[src + 1] + 0.05
      bp[dst + 2] = fp[src + 2]
    }
    const leftCount = segments + 1
    for (let i = 0; i <= segments; i++) {
      const src = (segments - i) * 2 * 3 + 3
      const dst = (leftCount + i) * 3
      bp[dst] = fp[src]
      bp[dst + 1] = fp[src + 1] + 0.05
      bp[dst + 2] = fp[src + 2]
    }

    fillGeo.attributes.position.needsUpdate = true
    borderGeo.attributes.position.needsUpdate = true
    fillGeo.setDrawRange(0, segments * 6)
    borderGeo.setDrawRange(0, vertCount)
    return true
  }

  let visible = $derived.by(() => {
    if (
      $editorTool !== 'road' ||
      !$editorHeightManager ||
      !$roadDrawStart ||
      !$brushWorldPos
    ) {
      return false
    }
    return updateStrip(
      $editorHeightManager,
      $roadDrawStart.x,
      $roadDrawStart.z,
      $brushWorldPos.x,
      $brushWorldPos.z,
      $brushSize
    )
  })

  onDestroy(() => {
    fillGeo.dispose()
    borderGeo.dispose()
  })
</script>

{#if visible}
  <T.Mesh renderOrder={999} frustumCulled={false}>
    <T is={fillGeo} />
    <T.MeshBasicMaterial
      color={PREVIEW_COLOR}
      transparent={true}
      opacity={0.35}
      side={THREE.DoubleSide}
      depthWrite={false}
    />
  </T.Mesh>
  <T.LineLoop renderOrder={999} frustumCulled={false}>
    <T is={borderGeo} />
    <T.LineBasicMaterial
      color={PREVIEW_COLOR}
      linewidth={2}
      depthWrite={false}
    />
  </T.LineLoop>
{/if}

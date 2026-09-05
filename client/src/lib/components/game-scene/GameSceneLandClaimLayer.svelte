<script lang="ts">
  import { T, useTask } from '@threlte/core'
  import * as THREE from 'three'
  import { onDestroy } from 'svelte'
  import {
    landClaimDialog,
    refreshLandClaimPreview,
  } from '../../stores/landClaimStore'
  import { networkManager } from '../../network/socket'
  import { createLandClaimMovementTracker } from './landClaimMovement'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'
  import { unwrapWorldXNear } from '../../terrain/world-wrap'
  import { LAND_PLOT_SIZE, TILE_DIM } from '../../terrain/terrain-constants'

  let {
    heightManager,
    playerPosition,
  }: {
    heightManager: TerrainHeightManager
    playerPosition: { x: number; z: number } | null
  } = $props()

  const geometry = new THREE.BufferGeometry()
  const positions = new THREE.Float32BufferAttribute(
    new Float32Array(5 * LAND_PLOT_SIZE * 4 * 3),
    3
  )
  geometry.setAttribute('position', positions)
  const material = new THREE.LineBasicMaterial({
    color: '#f2cb70',
    depthTest: false,
    transparent: true,
    opacity: 0.8,
  })
  const lines = new THREE.LineSegments(geometry, material)
  lines.renderOrder = 20
  lines.frustumCulled = false

  let elapsed = 0
  const shouldRefresh = createLandClaimMovementTracker()
  useTask((delta) => {
    const claim = $landClaimDialog
    if (shouldRefresh(claim, playerPosition, delta)) {
      refreshLandClaimPreview((id) => networkManager.sendUseItem(id))
    }
    lines.visible = claim !== null && playerPosition !== null
    if (!claim || !playerPosition) return
    elapsed += delta
    if (elapsed < 0.1) return
    elapsed = 0
    const minX = unwrapWorldXNear(
      playerPosition.x,
      claim.tile_x * TILE_DIM -
        LAND_PLOT_SIZE +
        (claim.quadrant % 2) * LAND_PLOT_SIZE
    )
    const minZ =
      claim.tile_z * TILE_DIM -
      LAND_PLOT_SIZE +
      Math.floor(claim.quadrant / 2) * LAND_PLOT_SIZE
    let vertex = 0
    const point = (x: number, z: number) =>
      positions.setXYZ(
        vertex++,
        x,
        heightManager.getHeightAtWorldPosition(x, z) + 0.12,
        z
      )
    for (
      let offset = 0;
      offset <= LAND_PLOT_SIZE;
      offset += LAND_PLOT_SIZE / 4
    ) {
      for (let step = 0; step < LAND_PLOT_SIZE; step++) {
        point(minX + offset, minZ + step)
        point(minX + offset, minZ + step + 1)
        point(minX + step, minZ + offset)
        point(minX + step + 1, minZ + offset)
      }
    }
    positions.needsUpdate = true
    material.color.set(
      claim.status === 'claimed'
        ? '#8ee080'
        : claim.status === 'rejected'
          ? '#ff4444'
          : '#f2cb70'
    )
  })

  onDestroy(() => {
    geometry.dispose()
    material.dispose()
  })
</script>

<T is={lines} />

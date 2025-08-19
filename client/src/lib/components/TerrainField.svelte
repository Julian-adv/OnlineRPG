<script lang="ts">
  import { T, useLoader } from '@threlte/core'
  import * as THREE from 'three'
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import { onMount } from 'svelte'

  interface Props {
    modelPath?: string
    gridSize?: number
    position?: [number, number, number]
    scale?: number | [number, number, number]
  }

  let {
    modelPath = '/models/3d_field_inspection.glb',
    gridSize = 3,
    position = [0, 0, 0],
    scale = 1,
  }: Props = $props()

  // Load GLB model
  const gltf = useLoader(GLTFLoader).load(modelPath)

  let terrainGroup = $state<THREE.Group | null>(null)
  let xSpacing = 53.4 // Will be calculated from bounding box
  let zSpacing = 27.7 // Will be calculated from bounding box

  function setupTerrain() {
    if ($gltf && !terrainGroup) {
      console.log('Setting up terrain field:', modelPath)

      console.log(
        `Using spacing: ${xSpacing.toFixed(2)} x ${zSpacing.toFixed(2)}`
      )

      const group = new THREE.Group()

      // Create grid of terrain models
      for (let x = 0; x < gridSize; x++) {
        for (let z = 0; z < gridSize; z++) {
          const cloned = $gltf.scene.clone()

          // Position each model in grid using calculated spacing
          const offsetX = (x - Math.floor(gridSize / 2)) * xSpacing
          const offsetZ = (z - Math.floor(gridSize / 2)) * zSpacing

          cloned.position.set(offsetX, 0, offsetZ)

          // Enable shadows on all meshes
          cloned.traverse((child) => {
            if (child instanceof THREE.Mesh) {
              child.castShadow = false // Terrain doesn't cast shadows
              child.receiveShadow = true // But can receive them
            }
          })

          group.add(cloned)
        }
      }

      terrainGroup = group
      console.log(
        `Created ${gridSize}x${gridSize} terrain grid with spacing ${xSpacing.toFixed(2)} x ${zSpacing.toFixed(2)}`
      )
    }
  }

  onMount(() => {
    // Wait for GLTF to load and setup terrain
    const checkGltf = () => {
      if ($gltf) {
        setupTerrain()
      } else {
        setTimeout(checkGltf, 100)
      }
    }
    checkGltf()

    return () => {
      terrainGroup = null
    }
  })
</script>

<!-- Terrain Grid -->
{#if terrainGroup}
  <T.Group
    {position}
    scale={typeof scale === 'number' ? [scale, scale, scale] : scale}
  >
    <T is={terrainGroup} />
  </T.Group>
{/if}

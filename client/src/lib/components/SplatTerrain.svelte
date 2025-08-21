<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { onMount } from 'svelte'
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import type { GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js'
  import { makeSplatStandardMaterial } from './makeSplatStandardMaterial'

  export let geometry: THREE.BufferGeometry // Pre-generated terrain geometry (e.g., a deformed PlaneGeometry)
  // Expose mesh reference to parent (for raycasting, etc.)
  export let mesh: THREE.Mesh | undefined = undefined

  let material: THREE.MeshStandardMaterial | null = null

  // Cache-bust splat map in dev so changes are reflected immediately
  const cacheBust = import.meta.env.DEV ? `?v=${Date.now()}` : ''

  // Adjust paths to fit your project
  const paths = {
    // Use a new filename to bypass any asset caching
    splat: `/textures/splat_rgba_v2.png${cacheBust}`, // RGBA weight map
    // GLB materials with tiled PBR textures applied to a plane
    // Adjusted mapping: rocky_terrain -> grass, gravel_floor -> rock
    grassGlb: '/textures/rocky_terrain_02_1k.glb',
    rockGlb: '/textures/gravel_floor_1k.glb',
    dirtGlb: '/textures/red_laterite_soil_stones_1k.glb',
    snowGlb: '/textures/snow_02_1k.glb',
  }

  onMount(async () => {
    const texLoader = new THREE.TextureLoader()
    const glbLoader = new GLTFLoader()

    const [splat, grassGlb, rockGlb, dirtGlb, snowGlb] = await Promise.all([
      texLoader.loadAsync(paths.splat),
      glbLoader.loadAsync(paths.grassGlb),
      glbLoader.loadAsync(paths.rockGlb),
      glbLoader.loadAsync(paths.dirtGlb),
      glbLoader.loadAsync(paths.snowGlb),
    ])

    function extractAlbedoMap(gltf: GLTF): THREE.Texture | null {
      let extracted: THREE.Texture | null = null
      gltf.scene.traverse((child) => {
        if (child instanceof THREE.Mesh) {
          const mat = child.material as THREE.Material | THREE.Material[] | undefined
          const first = Array.isArray(mat) ? mat[0] : mat
          if (first && first instanceof THREE.MeshStandardMaterial && first.map) {
            extracted = first.map
          }
        }
      })
      return extracted
    }

    const grass = extractAlbedoMap(grassGlb)!
    const rock = extractAlbedoMap(rockGlb)!
    const dirt = extractAlbedoMap(dirtGlb)!
    const snow = extractAlbedoMap(snowGlb)!

    material = makeSplatStandardMaterial({
      splatMap: splat,
      layers: [
        { map: grass, tile: 8.0 }, // R channel
        { map: rock, tile: 6.0 }, // G channel
        { map: dirt, tile: 10.0 }, // B channel
        { map: snow, tile: 4.0 }, // A channel
      ],
      splatScale: 1.0, // UV scale of the splat map (same ratio as terrain UVs)
    })
  })
</script>

{#if material}
  <T.Mesh bind:ref={mesh} {geometry} {material} castShadow receiveShadow />
{/if}

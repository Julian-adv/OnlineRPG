<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { onMount } from 'svelte'
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
    grass: '/textures/grass.png',
    rock: '/textures/rock.png',
    dirt: '/textures/dirt.png',
    snow: '/textures/snow.png',
  }

  onMount(async () => {
    const loader = new THREE.TextureLoader()

    const [splat, grass, rock, dirt, snow] = await Promise.all([
      loader.loadAsync(paths.splat),
      loader.loadAsync(paths.grass),
      loader.loadAsync(paths.rock),
      loader.loadAsync(paths.dirt),
      loader.loadAsync(paths.snow),
    ])

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

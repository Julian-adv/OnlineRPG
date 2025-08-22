<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { onMount } from 'svelte'
  // You can use either Addons.js (aggregate) or direct GLTFLoader import. Keep Addons for compatibility.
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import type { GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js'
  import {
    makeSplatStandardMaterial,
    type SplatLayer,
  } from './makeSplatStandardMaterial'

  export let geometry: THREE.BufferGeometry
  export let mesh: THREE.Mesh | undefined = undefined

  let material: THREE.MeshStandardMaterial | null = null

  // === Your assets ===
  const paths = {
    splat: '/textures/splat_rgba_v2.png',
    grassGlb: '/textures/rocky_terrain_02_1k.glb',
    rockGlb: '/textures/gravel_floor_1k.glb',
    dirtGlb: '/textures/red_laterite_soil_stones_1k.glb',
    snowGlb: '/textures/snow_02_1k.glb',
  }

  // --- helpers ---
  function prepColorTex(t: THREE.Texture | null) {
    if (!t) return null
    t.wrapS = t.wrapT = THREE.RepeatWrapping
    t.anisotropy = 8
    t.colorSpace = THREE.SRGBColorSpace
    t.needsUpdate = true
    return t
  }

  function prepDataTex(t: THREE.Texture | null) {
    if (!t) return null
    t.wrapS = t.wrapT = THREE.RepeatWrapping
    t.anisotropy = 8
    // keep Linear for non-color data
    t.needsUpdate = true
    return t
  }

  function firstMaterial(gltf: GLTF): THREE.MeshStandardMaterial | null {
    let found: THREE.MeshStandardMaterial | null = null
    gltf.scene.traverse((o: THREE.Object3D) => {
      if (found) return
      if (
        o instanceof THREE.Mesh &&
        o.material instanceof THREE.MeshStandardMaterial
      ) {
        found = o.material
      }
    })
    return found
  }

  // Pack AO(R) + MetallicRoughness(G,B) into one CanvasTexture (R=AO, G=Roughness, B=Metal)
  function packORM(
    ao: THREE.Texture | null,
    mr: THREE.Texture | null
  ): THREE.Texture | null {
    const aoImg = ao?.image as HTMLImageElement | undefined
    const mrImg = mr?.image as HTMLImageElement | undefined
    if (!aoImg && !mrImg) return null

    const w = mrImg?.width || aoImg?.width
    const h = mrImg?.height || aoImg?.height
    if (!w || !h) return null

    const canvas = document.createElement('canvas')
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')!
    // clear to defaults: AO=1, R=1; Rough=1, G=1; Metal=0, B=0
    ctx.fillStyle = 'rgb(255,255,0)'
    ctx.fillRect(0, 0, w, h)

    // Draw MR then AO to separate buffers to read pixels
    // MR: we need G,B channels
    if (mrImg) {
      const mrc = document.createElement('canvas')
      mrc.width = w
      mrc.height = h
      const mctx = mrc.getContext('2d')!
      mctx.drawImage(mrImg, 0, 0, w, h)
      const mrData = mctx.getImageData(0, 0, w, h).data

      const imgData = ctx.getImageData(0, 0, w, h)
      const data = imgData.data
      for (let i = 0; i < data.length; i += 4) {
        data[i + 1] = mrData[i + 1] // G = roughness
        data[i + 2] = mrData[i + 2] // B = metallic
      }
      ctx.putImageData(imgData, 0, 0)
    }

    if (aoImg) {
      const aoc = document.createElement('canvas')
      aoc.width = w
      aoc.height = h
      const actx = aoc.getContext('2d')!
      actx.drawImage(aoImg, 0, 0, w, h)
      const aoData = actx.getImageData(0, 0, w, h).data

      const imgData = ctx.getImageData(0, 0, w, h)
      const data = imgData.data
      for (let i = 0; i < data.length; i += 4) {
        data[i + 0] = aoData[i + 0] // R = AO
      }
      ctx.putImageData(imgData, 0, 0)
    }

    const tex = new THREE.CanvasTexture(canvas)
    tex.wrapS = tex.wrapT = THREE.RepeatWrapping
    tex.anisotropy = 8
    tex.flipY = false // match glTF loader behavior
    // Leave colorSpace as Linear for data
    tex.needsUpdate = true
    return tex
  }

  onMount(async () => {
    const loader = new THREE.TextureLoader()
    const glbLoader = new GLTFLoader()

    // Load splat first
    const splat = await loader.loadAsync(paths.splat)
    splat.wrapS = splat.wrapT = THREE.RepeatWrapping
    splat.minFilter = THREE.LinearMipMapLinearFilter
    splat.magFilter = THREE.LinearFilter
    splat.needsUpdate = true

    // Load GLBs (each contains one material we care about)
    const [grassGltf, rockGltf, dirtGltf, snowGltf] = await Promise.all([
      glbLoader.loadAsync(paths.grassGlb),
      glbLoader.loadAsync(paths.rockGlb),
      glbLoader.loadAsync(paths.dirtGlb),
      glbLoader.loadAsync(paths.snowGlb),
    ])

    function toLayer(gltf: GLTF, tile: number): SplatLayer {
      const mat = firstMaterial(gltf)
      if (!mat) throw new Error('No MeshStandardMaterial found in GLB')
      // Albedo
      const albedo = prepColorTex(mat.map || null)!
      // Normal
      const normal = prepDataTex(mat.normalMap || null) || undefined
      // MetallicRoughness (glTF packs both in one texture)
      // In three, either roughnessMap or metalnessMap will both point to the same texture when using glTF
      const mr = prepDataTex(mat.roughnessMap || mat.metalnessMap || null)
      // AO is separate in glTF
      const ao = prepDataTex(mat.aoMap || null)
      // Pack into a single ORM texture
      const orm = packORM(ao, mr) || undefined
      return { map: albedo, normalMap: normal, orm, tile }
    }

    const layers: [SplatLayer, SplatLayer, SplatLayer, SplatLayer] = [
      toLayer(grassGltf, 8.0), // R
      toLayer(rockGltf, 6.0), // G
      toLayer(dirtGltf, 10.0), // B
      toLayer(snowGltf, 4.0), // A
    ]

    material = makeSplatStandardMaterial({
      layers,
      splatMap: splat,
      splatScale: 1.0,
    })
  })
</script>

{#if material}
  <T.Mesh bind:ref={mesh} {geometry} {material} castShadow receiveShadow />
{/if}

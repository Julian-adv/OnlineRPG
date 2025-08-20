<script lang="ts">
  import { T, useLoader } from '@threlte/core'
  import * as THREE from 'three'
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import { onMount } from 'svelte'

  interface Props {
    gridSize?: number
    position?: [number, number, number]
    scale?: number | [number, number, number]
  }

  let {
    gridSize = 64,
    position = [0, 0, 0],
    scale = 1
  }: Props = $props()

  // Load material from GLB file
  const materialGltf = useLoader(GLTFLoader).load('/textures/gravel_road_1k.glb')
  
  let terrainMesh = $state<THREE.Mesh | null>(null)
  let gravelMaterial = $state<THREE.Material | null>(null)

  function createFlatTerrain() {
    // Create geometry for 64x64 grid
    const geometry = new THREE.PlaneGeometry(
      gridSize, // width
      gridSize, // height
      gridSize - 1, // width segments (63 segments for 64 vertices)
      gridSize - 1  // height segments (63 segments for 64 vertices)
    )
    
    // Adjust UV coordinates to repeat texture every 1m (gridSize times)
    const uvAttribute = geometry.attributes.uv
    const uvArray = uvAttribute.array as Float32Array
    
    for (let i = 0; i < uvArray.length; i += 2) {
      // Scale UV coordinates by gridSize to repeat texture gridSize times
      uvArray[i] = uvArray[i] * gridSize     // U coordinate
      uvArray[i + 1] = uvArray[i + 1] * gridSize // V coordinate
    }
    
    uvAttribute.needsUpdate = true
    
    // Rotate to lie flat on XZ plane
    geometry.rotateX(-Math.PI / 2)
    
    console.log(`Created terrain geometry: ${gridSize}x${gridSize} with ${geometry.attributes.position.count} vertices`)
    console.log(`UV coordinates scaled by ${gridSize} for 1m texture repeats`)
    
    return geometry
  }

  function extractMaterialFromGLB() {
    if ($materialGltf) {
      console.log('Extracting material from GLB file')
      
      // Find the material from the loaded GLB
      let extractedMaterial: THREE.Material | null = null
      
      $materialGltf.scene.traverse((child) => {
        if (child instanceof THREE.Mesh && child.material) {
          console.log(`Found mesh: ${child.name} with material: ${child.material.name || 'unnamed'}`)
          // Handle both single material and material array
          if (Array.isArray(child.material)) {
            extractedMaterial = child.material[0]
          } else {
            extractedMaterial = child.material
          }
        }
      })
      
      if (extractedMaterial) {
        // Clone the material to avoid modifying the original
        gravelMaterial = (extractedMaterial as THREE.Material).clone()
        
        // Set texture wrapping to repeat for all texture maps
        if (gravelMaterial instanceof THREE.MeshStandardMaterial) {
          if (gravelMaterial.map) {
            gravelMaterial.map.wrapS = THREE.RepeatWrapping
            gravelMaterial.map.wrapT = THREE.RepeatWrapping
          }
          if (gravelMaterial.normalMap) {
            gravelMaterial.normalMap.wrapS = THREE.RepeatWrapping
            gravelMaterial.normalMap.wrapT = THREE.RepeatWrapping
          }
          if (gravelMaterial.roughnessMap) {
            gravelMaterial.roughnessMap.wrapS = THREE.RepeatWrapping
            gravelMaterial.roughnessMap.wrapT = THREE.RepeatWrapping
          }
        }
        
        console.log('Successfully extracted and configured gravel road material')
      } else {
        console.warn('No material found in GLB file, using default material')
        gravelMaterial = new THREE.MeshStandardMaterial({ 
          color: 0x8B7355,
          roughness: 0.8,
          metalness: 0.1
        })
      }
    }
  }

  function setupTerrain() {
    if (gravelMaterial) {
      const geometry = createFlatTerrain()
      
      // Create mesh with extracted material
      const mesh = new THREE.Mesh(geometry, gravelMaterial)
      mesh.receiveShadow = true
      mesh.castShadow = false // Terrain doesn't cast shadows typically
      
      terrainMesh = mesh
      console.log('Terrain mesh created successfully')
    }
  }

  onMount(() => {
    // Wait for GLB to load, extract material, then create terrain
    const checkAndSetup = () => {
      if ($materialGltf) {
        extractMaterialFromGLB()
        setupTerrain()
      } else {
        setTimeout(checkAndSetup, 100)
      }
    }
    checkAndSetup()

    return () => {
      if (terrainMesh) {
        terrainMesh.geometry.dispose()
        if (Array.isArray(terrainMesh.material)) {
          terrainMesh.material.forEach(mat => mat.dispose())
        } else {
          terrainMesh.material.dispose()
        }
      }
    }
  })
</script>

<!-- Heightmap Terrain -->
{#if terrainMesh}
  <T.Group
    position={position}
    scale={typeof scale === 'number' ? [scale, scale, scale] : scale}
  >
    <T is={terrainMesh} />
  </T.Group>
{/if}
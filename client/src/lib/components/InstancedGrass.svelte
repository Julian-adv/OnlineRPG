<script lang="ts">
  import { T, useLoader } from '@threlte/core'
  import { InstancedMesh, Object3D, Mesh } from 'three'
  import * as THREE from 'three'
  import { GLTFLoader } from 'three/examples/jsm/Addons.js'
  import { onMount } from 'svelte'

  // InstancedMesh for grass
  let grassInstancedMeshes = $state<InstancedMesh[]>([])
  
  // Animated grass mixer
  let grassMixer: THREE.AnimationMixer | undefined

  // Grass position type
  interface GrassPosition {
    id: number
    x: number
    y: number
    z: number
    rotation: number
    grassType: number
    scale: number
  }

  // Grass bounding box type
  interface GrassBoundingBox {
    width: number
    depth: number
    height: number
    filename?: string
    vertexCount?: number
  }

  // JSON mesh data type
  interface MeshData {
    boundingBox: {
      size: {
        width: number
        height: number
        depth: number
      }
    }
    filename: string
    vertexCount: number
  }

  // Load individual grass models
  const grass1 = useLoader(GLTFLoader).load('/models/grass_1_Object_4.glb')
  const grass2 = useLoader(GLTFLoader).load('/models/grass_2_Object_6.glb')
  const grass3 = useLoader(GLTFLoader).load('/models/grass_3_Object_8.glb')
  const grass4 = useLoader(GLTFLoader).load('/models/grass_4_Object_10.glb')
  const grass5 = useLoader(GLTFLoader).load('/models/grass_5_Object_12.glb')
  const grass6 = useLoader(GLTFLoader).load('/models/grass_6_Object_14.glb')
  const grass7 = useLoader(GLTFLoader).load('/models/grass_7_Object_16.glb')
  const grass8 = useLoader(GLTFLoader).load('/models/grass_8_Object_18.glb')
  const grass9 = useLoader(GLTFLoader).load('/models/grass_9_Object_20.glb')
  
  // Load animated grass model
  const grassBrushA = useLoader(GLTFLoader).load('/models/grass_bursh_displacement_a_eo_001.glb')

  function setupInstancedGrass() {
    const instancedMeshes: InstancedMesh[] = []

    // Group positions by grass type
    const positionsByType: GrassPosition[][] = Array(9)
      .fill(null)
      .map(() => [])
    grassPositions.forEach((pos) => {
      positionsByType[pos.grassType].push(pos)
    })

    // Create InstancedMesh for each grass type
    const grassModels = [
      $grass1,
      $grass2,
      $grass3,
      $grass4,
      $grass5,
      $grass6,
      $grass7,
      $grass8,
      $grass9,
    ]

    grassModels.forEach((model, index) => {
      if (model && positionsByType[index].length > 0) {
        const positions = positionsByType[index]
        const count = positions.length

        // Get the first mesh from the GLTF model
        let foundMesh: Mesh | null = null
        model.scene.traverse((child) => {
          if (child instanceof Mesh && !foundMesh) {
            foundMesh = child
          }
        })

        if (foundMesh) {
          // Create InstancedMesh with explicit type assertion
          const mesh = foundMesh as Mesh
          const instancedMesh = new InstancedMesh(
            mesh.geometry,
            mesh.material,
            count
          )

          instancedMesh.castShadow = false
          instancedMesh.receiveShadow = true

          // Create matrices for each instance
          const dummy = new Object3D()

          positions.forEach((pos, i) => {
            dummy.position.set(pos.x, 0.0, pos.z)
            dummy.rotation.set(0, pos.rotation, 0)
            dummy.scale.setScalar(pos.scale)
            dummy.updateMatrix()

            instancedMesh.setMatrixAt(i, dummy.matrix)
          })

          instancedMesh.instanceMatrix.needsUpdate = true

          instancedMeshes.push(instancedMesh)
        }
      }
    })

    grassInstancedMeshes = instancedMeshes
    console.log(`Created ${instancedMeshes.length} InstancedMesh objects`)
  }

  // Load grass bounding box data from JSON file
  let grassBoundingBoxes = $state<GrassBoundingBox[]>([])
  let grassPositionsReady = $state(false)

  async function loadGrassBoundingBoxes() {
    try {
      const response = await fetch('/models/grass_meshes_info.json')
      const data = await response.json()
      const boundingBoxes = data.meshes.map((mesh: MeshData) => ({
        width: mesh.boundingBox.size.width,
        depth: mesh.boundingBox.size.depth,
        height: mesh.boundingBox.size.height,
        filename: mesh.filename,
        vertexCount: mesh.vertexCount,
      }))

      console.log('Loaded grass bounding boxes:', boundingBoxes)
      grassBoundingBoxes = boundingBoxes
      grassPositionsReady = true
      return boundingBoxes
    } catch (error) {
      console.warn(
        'Could not load grass_meshes_info.json, using default values:',
        error
      )
      // Fallback to default values
      grassBoundingBoxes = [
        { width: 0.8, depth: 0.8, height: 0.3 },
        { width: 0.6, depth: 0.6, height: 0.3 },
        { width: 0.9, depth: 0.9, height: 0.3 },
        { width: 0.7, depth: 0.7, height: 0.3 },
        { width: 0.5, depth: 0.5, height: 0.3 },
        { width: 0.8, depth: 0.8, height: 0.3 },
        { width: 0.6, depth: 0.6, height: 0.3 },
        { width: 0.7, depth: 0.7, height: 0.3 },
        { width: 0.9, depth: 0.9, height: 0.3 },
      ]
      grassPositionsReady = true
      return grassBoundingBoxes
    }
  }

  // Generate grass positions with 10x10 blocks for each grass type
  function generateGrassPositions() {
    if (!grassPositionsReady || grassBoundingBoxes.length === 0) {
      console.log(
        'Grass bounding boxes not ready yet, returning empty positions'
      )
      return []
    }

    const positions = []
    let id = 0
    let currentBlockStartX = 0

    // Generate 10x10 blocks for each grass type (0-8)
    for (let grassType = 0; grassType < 9; grassType++) {
      const boundingBox = grassBoundingBoxes[grassType]
      if (!boundingBox) {
        console.warn(`No bounding box data for grass type ${grassType}`)
        continue
      }

      const spacingX = boundingBox.width * 0.9 // 5% gap between meshes on X axis
      const spacingZ = boundingBox.depth * 0.9 // 5% gap between meshes on Z axis

      // Calculate block dimensions
      const blockWidth = spacingX * 10
      const blockDepth = spacingZ * 10
      const blockStartX = currentBlockStartX
      const blockStartZ = -blockDepth / 2 // Center the block on Z axis

      console.log(
        `Generating grass type ${grassType + 1} (${boundingBox.filename || 'unknown'}) block at X: ${blockStartX.toFixed(2)}, spacingX: ${spacingX.toFixed(3)}, spacingZ: ${spacingZ.toFixed(3)}, size: ${boundingBox.width.toFixed(3)}x${boundingBox.depth.toFixed(3)}`
      )

      // Generate 10x10 grid for this grass type
      for (let x = 0; x < 10; x++) {
        for (let z = 0; z < 10; z++) {
          const posX = blockStartX + x * spacingX + spacingX / 2
          const posZ = blockStartZ + z * spacingZ + spacingZ / 2

          positions.push({
            id: id++,
            x: posX,
            y: 0,
            z: posZ,
            rotation: Math.random() * Math.PI * 2, // Random rotation for variety
            grassType: grassType,
            scale: 1.0 + (Math.random() - 0.5) * 0.3, // Scale variation ±15%
          })
        }
      }

      // Move to next block position (with some gap between blocks)
      currentBlockStartX += blockWidth + spacingX * 2
    }

    // Generate additional 100x100 random grass field at (10,0,10)
    const minBoundingSize = Math.min(
      ...grassBoundingBoxes.map((bb) => Math.min(bb.width, bb.depth))
    )
    const randomSpacing = minBoundingSize * 1.0 // Use smallest bounding box size with 10% gap

    const randomFieldStartX = 10
    const randomFieldStartZ = 10
    const randomFieldSize = 100 // 100x100 grid

    console.log(
      `Adding random grass field at (${randomFieldStartX}, 0, ${randomFieldStartZ}) with spacing: ${randomSpacing.toFixed(3)}`
    )

    for (let x = 0; x < randomFieldSize; x++) {
      for (let z = 0; z < randomFieldSize; z++) {
        const posX = randomFieldStartX + x * randomSpacing
        const posZ = randomFieldStartZ + z * randomSpacing

        positions.push({
          id: id++,
          x: posX,
          y: 0,
          z: posZ,
          rotation: Math.random() * Math.PI * 2, // Random rotation
          grassType: Math.floor(Math.random() * 9), // Random grass type (0-8)
          scale: 1.0 + (Math.random() - 0.5) * 0.4, // Scale variation ±20%
        })
      }
    }

    console.log(`Generated ${positions.length} grass positions:`)
    console.log(
      `- ${grassBoundingBoxes.length} organized blocks (${grassBoundingBoxes.length * 100} grass)`
    )
    console.log(`- 1 random field (${randomFieldSize * randomFieldSize} grass)`)
    console.log(
      `Total organized area width: ${currentBlockStartX.toFixed(2)} units`
    )
    console.log(
      `Random field area: ${randomFieldSize * randomSpacing}x${randomFieldSize * randomSpacing} units`
    )
    return positions
  }

  // Initialize grass positions as reactive variable
  let grassPositions = $state<GrassPosition[]>([])

  // Effect to regenerate positions when bounding boxes are loaded
  $effect(() => {
    if (grassPositionsReady) {
      grassPositions = generateGrassPositions()
    }
  })

  // Effect to setup instanced grass when both models and positions are ready
  $effect(() => {
    const models = [
      $grass1,
      $grass2,
      $grass3,
      $grass4,
      $grass5,
      $grass6,
      $grass7,
      $grass8,
      $grass9,
    ]
    const allModelsLoaded = models.every((model) => model !== null)

    if (allModelsLoaded && grassPositions.length > 0) {
      console.log(
        'Both grass models and positions ready, setting up InstancedMesh...'
      )
      setupInstancedGrass()
    }
  })

  // Export mixer update function for parent component
  export function updateMixer(deltaTime: number) {
    if (grassMixer) {
      grassMixer.update(deltaTime / 1000)
    }
  }

  onMount(() => {
    // Load grass bounding box data first
    loadGrassBoundingBoxes()

    // Setup animated grass mixer
    const checkAndSetupGrassAnimation = () => {
      if ($grassBrushA && !grassMixer && $grassBrushA.animations?.length > 0) {
        grassMixer = new THREE.AnimationMixer($grassBrushA.scene)
        const action = grassMixer.clipAction($grassBrushA.animations[0])
        action.play()
        console.log('Animated grass started:', $grassBrushA.animations[0].name)
      }
    }
    
    // Check immediately and periodically until setup
    checkAndSetupGrassAnimation()
    const grassSetupInterval = setInterval(() => {
      checkAndSetupGrassAnimation()
      if (grassMixer) {
        clearInterval(grassSetupInterval)
      }
    }, 100)

    return () => {
      if (grassSetupInterval) {
        clearInterval(grassSetupInterval)
      }
    }
  })
</script>

<!-- Instanced Grass Meshes -->
{#each grassInstancedMeshes as instancedMesh, index (index)}
  <T is={instancedMesh} />
{/each}

<!-- Animated Grass at (0, 0, 32) -->
{#if $grassBrushA}
  <T is={$grassBrushA.scene} position={[0, 0, 32]} rotation={[0, 0, 0]} />
{/if}
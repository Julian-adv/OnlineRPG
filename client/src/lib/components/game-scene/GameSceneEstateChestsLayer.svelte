<script lang="ts">
  import { T, useTask, useThrelte } from '@threlte/core'
  import { onMount } from 'svelte'
  import { SvelteMap } from 'svelte/reactivity'
  import { get } from 'svelte/store'
  import * as THREE from 'three'
  import GameSceneEstateFurniturePlacementLayer from './GameSceneEstateFurniturePlacementLayer.svelte'
  import { loadGLB } from '../../utils/gltfCache'
  import { networkManager } from '../../network/socket'
  import { playPropSound } from '../../managers/sfxManager'
  import {
    estateChests,
    estateChestMode,
    estateChestPending,
    estateChestError,
    openEstateChest,
    stopEstateChestMode,
  } from '../../stores/estateStorageStore'
  import {
    mapEditorMode,
    housingEditorMode,
    cameraRotationEnabled,
  } from '../../stores/debugStore'
  import { currentDungeonDepth } from '../../stores/dungeonStore'
  import { playerVisualFloorLevel } from '../../stores/housingStore'
  import { unwrapWorldXNear } from '../../terrain/world-wrap'
  import type { LocalPlayer } from '../../stores/gameStore'
  import type { EstateChest } from '../../network/networkTypes'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'
  import type { EstateFurniturePlacement } from '../../terrain/estatePlacement'
  import {
    estateStorageDefs,
    getEstateStorageDef,
  } from '../../data/estateFurnitureDefs'

  let {
    heightManager,
    terrainMeshes,
    housingGroup,
    player,
  }: {
    heightManager: TerrainHeightManager
    terrainMeshes: (THREE.Mesh | undefined)[]
    housingGroup?: THREE.Group
    player: LocalPlayer | null
  } = $props()

  const { camera, renderer } = useThrelte()
  const group = new THREE.Group()
  const chestGroup = new THREE.Group()
  const raycaster = new THREE.Raycaster()
  const pointer = new THREE.Vector2()
  group.add(chestGroup)

  const sources = new SvelteMap<
    string,
    { scene: THREE.Group; clips: THREE.AnimationClip[] }
  >()
  let lastChests: Map<number, EstateChest> | null = null
  let lastWrapX = Infinity
  let lastFloorLevel = Infinity
  let disposed = false
  let lastOpened: number | null = null
  const visuals = new SvelteMap<number, THREE.Group>()
  const mixers = new SvelteMap<number, THREE.AnimationMixer>()
  const placementDefinition = $derived(
    getEstateStorageDef($estateChestMode?.item_def_id)
  )
  const placementObstacles = $derived(
    [...$estateChests.values()]
      .filter((chest) => chest.floor_level === $playerVisualFloorLevel)
      .flatMap((chest) => {
        const definition = getEstateStorageDef(chest.item_def_id)
        return definition
          ? [
              {
                x: chest.position.x,
                z: chest.position.z,
                rotationDeg: chest.rotation_deg,
                footprint: definition.footprint,
              },
            ]
          : []
      })
  )

  function setPointer(clientX: number, clientY: number) {
    const rect = renderer.domElement.getBoundingClientRect()
    pointer.set(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -((clientY - rect.top) / rect.height) * 2 + 1
    )
    raycaster.setFromCamera(pointer, get(camera))
  }

  function chestFromHit(hit: THREE.Intersection): EstateChest | undefined {
    let object: THREE.Object3D | null = hit.object
    while (object) {
      if (typeof object.userData.estateChestId === 'number')
        return get(estateChests).get(object.userData.estateChestId)
      object = object.parent
    }
  }

  function makeVisual(chest: EstateChest) {
    const source = sources.get(chest.item_def_id)
    if (!source || !player) return
    const visual = source.scene.clone(true)
    visual.userData.estateChestId = chest.id
    visual.position.set(
      unwrapWorldXNear(player.position.x, chest.position.x),
      chest.position.y,
      chest.position.z
    )
    visual.rotation.y = THREE.MathUtils.degToRad(chest.rotation_deg)
    visual.traverse((object) => {
      if (object instanceof THREE.Mesh) {
        object.castShadow = true
        object.receiveShadow = true
      }
    })
    chestGroup.add(visual)
    visuals.set(chest.id, visual)
    mixers.set(chest.id, new THREE.AnimationMixer(visual))
  }

  function rebuild() {
    const current = get(estateChests)
    const floorLevel = get(playerVisualFloorLevel)
    if (
      sources.size === 0 ||
      !player ||
      (current === lastChests &&
        floorLevel === lastFloorLevel &&
        Math.abs(player.position.x - lastWrapX) < 100)
    )
      return
    lastChests = current
    lastWrapX = player.position.x
    lastFloorLevel = floorLevel
    chestGroup.clear()
    visuals.clear()
    mixers.clear()
    for (const chest of current.values()) {
      if (chest.floor_level === floorLevel) makeVisual(chest)
    }
  }

  function place(placement: EstateFurniturePlacement) {
    const mode = get(estateChestMode)
    if (!mode || !getEstateStorageDef(mode.item_def_id)) return
    estateChestPending.set(true)
    estateChestError.set(null)
    networkManager.sendPlaceEstateChest(
      mode.instance_id,
      placement.position,
      placement.rotationDeg,
      placement.floorLevel
    )
  }

  function playOpen(chestId: number) {
    const chest = get(estateChests).get(chestId)
    const clips = chest ? sources.get(chest.item_def_id)?.clips : undefined
    const clip =
      clips?.find((entry) => entry.name === 'ChestOpen') ?? clips?.[0]
    const mixer = mixers.get(chestId)
    if (!clip || !mixer) return
    const action = mixer.clipAction(clip)
    action.reset()
    action.setLoop(THREE.LoopOnce, 1)
    action.clampWhenFinished = true
    action.timeScale = 1
    action.play()
    playPropSound('chestOpen')
  }

  function playClose(chestId: number) {
    const chest = get(estateChests).get(chestId)
    const clips = chest ? sources.get(chest.item_def_id)?.clips : undefined
    const clip =
      clips?.find((entry) => entry.name === 'ChestOpen') ?? clips?.[0]
    const mixer = mixers.get(chestId)
    if (!clip || !mixer) return
    const action = mixer.clipAction(clip)
    action.enabled = true
    action.paused = false
    action.setLoop(THREE.LoopOnce, 1)
    action.clampWhenFinished = true
    action.timeScale = -1
    if (action.time <= 0) action.time = clip.duration
    action.play()
  }

  onMount(() => {
    for (const definition of estateStorageDefs.values()) {
      loadGLB(definition.modelUrl)
        .then((gltf) => {
          if (disposed) return
          sources.set(definition.itemDefId, {
            scene: gltf.scene,
            clips: gltf.animations,
          })
          lastChests = null
          rebuild()
        })
        .catch((error) => {
          console.error(
            `Failed to load estate storage chest ${definition.itemDefId}:`,
            error
          )
          estateChestError.set(
            'Could not load the chest model. Reload to try again.'
          )
        })
    }

    const canvas = renderer.domElement
    const click = (event: MouseEvent) => {
      if (
        get(estateChestMode) ||
        !player ||
        (event.button !== 0 && event.button !== 2) ||
        get(cameraRotationEnabled) ||
        get(mapEditorMode) ||
        get(housingEditorMode) ||
        get(currentDungeonDepth) > 0
      )
        return
      setPointer(event.clientX, event.clientY)
      const hit = raycaster.intersectObject(chestGroup, true)[0]
      const chest = hit && chestFromHit(hit)
      if (
        !chest ||
        chest.floor_level !== get(playerVisualFloorLevel) ||
        Math.hypot(
          unwrapWorldXNear(player.position.x, chest.position.x) -
            player.position.x,
          chest.position.z - player.position.z
        ) > 3
      )
        return
      event.preventDefault()
      event.stopImmediatePropagation()
      if (event.button === 0) networkManager.sendOpenEstateChest(chest.id)
      else networkManager.sendRecoverEstateChest(chest.id)
    }
    canvas.addEventListener('mousedown', click, true)
    return () => {
      disposed = true
      canvas.removeEventListener('mousedown', click, true)
      stopEstateChestMode()
    }
  })

  useTask((delta) => {
    group.visible = get(currentDungeonDepth) < 1
    rebuild()
    for (const mixer of mixers.values()) mixer.update(delta)
    const opened = get(openEstateChest)?.chest_id ?? null
    if (opened !== lastOpened) {
      if (lastOpened !== null) playClose(lastOpened)
      if (opened !== null) playOpen(opened)
      lastOpened = opened
    }
  })
</script>

<T is={group} />
{#if placementDefinition}
  <GameSceneEstateFurniturePlacementLayer
    definition={placementDefinition}
    active={$estateChestMode !== null}
    plots={$estateChestMode?.plots ?? []}
    pending={$estateChestPending}
    {terrainMeshes}
    {housingGroup}
    {heightManager}
    {player}
    floorLevel={$playerVisualFloorLevel}
    obstacles={placementObstacles}
    onplace={place}
    oncancel={stopEstateChestMode}
    onerror={(message) => estateChestError.set(message)}
  />
{/if}

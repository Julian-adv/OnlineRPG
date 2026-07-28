<script lang="ts">
  import * as THREE from 'three'
  import { T } from '@threlte/core'
  import { onDestroy } from 'svelte'
  import {
    editorTool,
    currentObjectData,
    objectCatalog,
    selectedObjectPlacementId,
    objectPreviewPos,
    objectRotation,
    selectedObjectType,
  } from '../../stores/editorStore'
  import type {
    EditorTool,
    ObjectDef,
    ObjectPlacement,
  } from '../../stores/editorStore'
  import { playerDebugInfo } from '../../stores/debugStore'
  import type { PlayerDebugInfo } from '../../stores/debugStore'
  import { mapEditorMode } from '../../stores/debugStore'
  import { tileToRegion } from '../../terrain/terrain-constants'
  import { TERRAIN_TILE_SIZE } from '../game-scene/terrain-utils'
  import { objectManager } from '../../managers/objectManager'
  import { bridgeManager } from '../../managers/bridgeManager'
  import { furnitureManager } from '../../managers/furnitureManager'
  import {
    playerFloorLevel,
    playerInsideHouseId,
  } from '../../stores/housingStore'
  import { housingManager } from '../../managers/housingManager'
  import { loadGLB } from '../../utils/gltfCache'
  import { getObjectModelPath } from '../../utils/modelPaths'
  import { buildShopSignBoard, buildShopSignText } from '../../utils/shop-sign'
  import type { Unsubscriber } from 'svelte/store'
  import { SvelteMap, SvelteSet } from 'svelte/reactivity'

  const HIGHLIGHT_COLOR = new THREE.Color(0x44ccff)
  const PREVIEW_OPACITY = 0.5
  const SELECTION_OPACITY = 0.9
  const SELECTION_RENDER_ORDER = 999
  const GHOST_OPACITY = 0.3

  let tool = $state<EditorTool>('height')
  let placements = $state<ObjectPlacement[]>([])
  let selectedId = $state<number | null>(null)
  let previewPos = $state<{ x: number; y: number; z: number } | null>(null)
  let rotation = $state(0)
  let selectedType = $state<string | null>(null)
  let debugInfo = $state<PlayerDebugInfo | null>(null)
  let isEditorMode = $state(false)
  let currentFloor = $state(0)
  let currentHouseId = $state<string | null>(null)

  let catalogById = new Map<string, ObjectDef>()

  let lastLoadedRegion = { rx: NaN, rz: NaN }

  const unsubs: Unsubscriber[] = [
    editorTool.subscribe((v) => (tool = v)),
    currentObjectData.subscribe((v) => {
      placements = v.placements
      // Re-sync bridge + furniture collision so newly placed/edited objects
      // take effect immediately. Guard for the initial empty-state fire before
      // catalog loads (and before any region has been loaded).
      if (catalogById.size > 0) {
        bridgeManager.syncRegion(v.placements, catalogById)
      }
      if (!Number.isNaN(lastLoadedRegion.rx)) {
        furnitureManager.syncRegion(
          lastLoadedRegion.rx,
          lastLoadedRegion.rz,
          v.placements
        )
      }
    }),
    objectCatalog.subscribe((v) => {
      // Keep catalogById in sync with whoever populated the store
      // (ObjectBrushPanel can fetch the catalog before loadRegionObject runs).
      catalogById = new Map(v.map((d) => [d.id, d]))
    }),
    selectedObjectPlacementId.subscribe((v) => (selectedId = v)),
    objectPreviewPos.subscribe((v) => (previewPos = v)),
    objectRotation.subscribe((v) => (rotation = v)),
    selectedObjectType.subscribe((v) => (selectedType = v)),
    playerDebugInfo.subscribe((v) => (debugInfo = v)),
    mapEditorMode.subscribe((v) => (isEditorMode = v)),
    playerFloorLevel.subscribe((v) => (currentFloor = v)),
    playerInsideHouseId.subscribe((v) => (currentHouseId = v)),
  ]
  onDestroy(() => unsubs.forEach((u) => u()))

  async function loadRegionObject(rx: number, rz: number) {
    if (rx === lastLoadedRegion.rx && rz === lastLoadedRegion.rz) return
    lastLoadedRegion = { rx, rz }

    if (catalogById.size === 0) {
      const cat = await objectManager.fetchCatalog()
      objectCatalog.set(cat)
    }

    const data = await objectManager.fetchObject(rx, rz)
    // A newer region load may have superseded this one while awaiting (fast
    // region crossing with out-of-order fetch resolution). If so, drop this
    // stale result: otherwise currentObjectData.set fires the subscription,
    // which syncs furniture for the *current* lastLoadedRegion — mis-keying
    // this region's cells under the wrong region — and shows stale objects.
    if (rx !== lastLoadedRegion.rx || rz !== lastLoadedRegion.rz) return
    // currentObjectData.set fires the subscription above synchronously, which
    // syncs furniture for lastLoadedRegion (== rx,rz here) — no direct call needed.
    currentObjectData.set(data)
    bridgeManager.syncRegion(data.placements, catalogById)
    // Sync first, evict after: the new region is in place before its distant
    // neighbours go, so collision is never momentarily absent underfoot.
    furnitureManager.evictDistant(rx, rz)
  }

  $effect(() => {
    if (!debugInfo) return
    const tileX = Math.round(debugInfo.position.x / TERRAIN_TILE_SIZE)
    const tileZ = Math.round(debugInfo.position.z / TERRAIN_TILE_SIZE)
    const rx = tileToRegion(tileX)
    const rz = tileToRegion(tileZ)
    loadRegionObject(rx, rz)
  })

  const modelCache = new SvelteMap<string, THREE.Group>()
  const modelBounds = new SvelteMap<
    string,
    { center: THREE.Vector3; size: THREE.Vector3 }
  >()
  const loadingModels = new SvelteSet<string>()

  /** Build a procedural object template (text-less; text is added per instance
   *  in rebuild). Returns null for unknown builder ids. */
  function buildProceduralModel(kind: string): THREE.Group | null {
    if (kind === 'shopSign') return buildShopSignBoard()
    return null
  }

  /** Cache of baked sign-text meshes keyed by `type\0text`. buildShopSignText
   *  allocates a CanvasTexture + node material + geometry that the disposer must
   *  skip (WebGPU sampler crash on dispose), so building one per rebuild would
   *  leak GPU memory on every editor interaction (dragging the Rot slider fires
   *  rebuild() dozens of times/sec). Build once per unique text and hand out
   *  clones — a Mesh clone shares geometry/material/texture, so no new GPU
   *  resources are allocated. */
  const signTextCache = new SvelteMap<string, THREE.Mesh>()

  function getSignText(type: string, text: string): THREE.Object3D {
    const key = `${type}\u0000${text}`
    let base = signTextCache.get(key)
    if (!base) {
      base = buildShopSignText(text)
      signTextCache.set(key, base)
    }
    // Clone shares the cached geometry/material/texture; a Mesh can only have
    // one parent, so each placement needs its own lightweight clone.
    return base.clone()
  }

  async function getModel(objectId: string): Promise<THREE.Group | null> {
    if (modelCache.has(objectId)) return modelCache.get(objectId)!
    if (loadingModels.has(objectId)) return null

    const def = catalogById.get(objectId)
    if (!def) return null

    loadingModels.add(objectId)
    try {
      let model: THREE.Group | null
      if (def.procedural) {
        // Procedural objects (e.g. shop signs) build their geometry in code and
        // register it into the same template cache the GLB path uses, so
        // cloning, preview, selection box and per-instance text all work
        // unchanged. Defer past any in-progress rebuild() before mutating the
        // cache and re-running it, exactly as the GLB path's `await loadGLB`
        // does — otherwise a synchronous rebuild() call from inside rebuild()'s
        // own loop would re-enter and duplicate placements.
        await Promise.resolve()
        model = buildProceduralModel(def.procedural)
      } else {
        if (!def.model) return null
        const gltf = await loadGLB(getObjectModelPath(def.model))
        // Bridges ray-cast against the cached, untransformed scene at runtime
        // so the player Y tracks the actual deck curve precisely.
        if (def.kind === 'bridge') {
          bridgeManager.registerBridgeMesh(objectId, gltf.scene)
        }
        model = gltf.scene.clone()
        model.traverse((child) => {
          if (child instanceof THREE.Mesh) {
            child.castShadow = true
            child.receiveShadow = true
          }
        })
      }
      if (!model) return null

      const box = new THREE.Box3().setFromObject(model)
      const center = new THREE.Vector3()
      const size = new THREE.Vector3()
      box.getCenter(center)
      box.getSize(size)
      modelBounds.set(objectId, { center, size })
      modelCache.set(objectId, model)
      // Force a full rebuild (not the transform-only fast path) so the newly
      // loaded template actually gets cloned into the scene.
      lastBuildKey = ''
      lastStructKey = ''
      rebuild()
      // If the user is currently previewing this object, build the preview now —
      // otherwise the cursor stays empty until the mouse moves again.
      if (selectedType === objectId) updatePreview()
      return model
    } finally {
      loadingModels.delete(objectId)
    }
  }

  let group = new THREE.Group()
  group.name = 'object-overlay'

  let previewGroup: THREE.Group | null = null
  let previewType: string | null = null

  function disposeClonedMaterials(obj: THREE.Object3D) {
    obj.traverse((child) => {
      // Shop-sign text uses MeshBasicNodeMaterial + CanvasTexture, whose sampler
      // bindings crash the WebGPU backend if disposed (see TextLabel.svelte and
      // shop-sign.ts). The board reuses the shared housing door material, which
      // other houses depend on. Leave both for GC / shared ownership.
      if (child.userData?.isSignText || child.userData?.isSignBoard) return
      if (child instanceof THREE.Mesh && child.material) {
        child.material.dispose()
      } else if (child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        ;(child.material as THREE.Material).dispose()
      }
    })
  }

  function createSelectionBox(
    center: THREE.Vector3,
    size: THREE.Vector3
  ): THREE.LineSegments {
    const box = new THREE.BoxGeometry(size.x, size.y, size.z)
    const geo = new THREE.EdgesGeometry(box)
    box.dispose()
    const mat = new THREE.LineBasicMaterial({
      color: HIGHLIGHT_COLOR,
      depthTest: false,
      transparent: true,
      opacity: SELECTION_OPACITY,
    })
    const lines = new THREE.LineSegments(geo, mat)
    lines.position.copy(center)
    lines.renderOrder = SELECTION_RENDER_ORDER
    return lines
  }

  function setPreviewMaterial(obj: THREE.Object3D, opacity: number) {
    obj.traverse((child) => {
      if (child instanceof THREE.Mesh) {
        child.material = (child.material as THREE.Material).clone()
        ;(child.material as THREE.Material).transparent = true
        ;(child.material as THREE.Material).opacity = opacity
        ;(child.material as THREE.Material).depthWrite = false
      }
    })
  }

  let lastBuildKey = ''
  let lastStructKey = ''
  /** objectId → its placement clone in `group`, so the transform-only fast path
   *  can move an existing clone instead of tearing the whole region down. */
  const cloneById = new SvelteMap<number, THREE.Object3D>()
  const isEditing = () => isEditorMode && tool === 'object'

  /** Apply a placement's position + rotation (yaw + pitch) to its clone. Single
   *  source of transform threading for both the full rebuild and fast path. */
  function applyPlacementTransform(clone: THREE.Object3D, p: ObjectPlacement) {
    clone.position.set(p.x, p.y, p.z)
    clone.rotation.set(
      ((p.rotationX ?? 0) * Math.PI) / 180,
      (p.rotation * Math.PI) / 180,
      0
    )
  }

  /** Placements currently shown, after floor + house filtering. */
  function visiblePlacements(visibleFloor: number): ObjectPlacement[] {
    return placements.filter((p) => {
      if (p.floorLevel !== visibleFloor) return false
      const pHouse = housingManager.findHouseAtPoint(p.x, p.y, p.z)
      return currentHouseId ? pHouse?.id === currentHouseId : pHouse == null
    })
  }

  function rebuild() {
    const visible = visiblePlacements(currentFloor)
    // Split the change signature: `structKey` is everything that changes WHICH
    // clones exist (id/type/text set, selection, floor, house); `transformKey`
    // is just each clone's position/rotation. When only transforms changed we
    // move the existing clones in place instead of teardown + re-clone — crucial
    // on WebGPU, where re-instantiating meshes/materials rebuilds pipelines and
    // bind groups and tanks FPS during a slider/wheel drag.
    const structKey =
      visible.map((p) => `${p.id}:${p.type}:${p.text ?? ''}`).join('|') +
      `|sel:${isEditing() ? selectedId : ''}|fl:${currentFloor}|h:${currentHouseId ?? ''}`
    const transformKey = visible
      .map(
        (p) => `${p.id}:${p.x}:${p.y}:${p.z}:${p.rotation}:${p.rotationX ?? 0}`
      )
      .join('|')
    const key = structKey + '||' + transformKey
    if (key === lastBuildKey) return
    const structUnchanged = structKey === lastStructKey && lastStructKey !== ''
    lastBuildKey = key
    lastStructKey = structKey

    if (structUnchanged) {
      for (const p of visible) {
        const clone = cloneById.get(p.id)
        if (clone) applyPlacementTransform(clone, p)
      }
      return
    }

    cloneById.clear()
    for (let i = group.children.length - 1; i >= 0; i--) {
      const child = group.children[i]
      if (child !== previewGroup) {
        disposeClonedMaterials(child)
        group.remove(child)
      }
    }

    for (const p of visible) {
      const template = modelCache.get(p.type)
      if (!template) {
        getModel(p.type)
        continue
      }
      const clone = template.clone()
      applyPlacementTransform(clone, p)
      if (isEditing() && p.id === selectedId) {
        const bounds = modelBounds.get(p.type)
        if (bounds) {
          clone.add(createSelectionBox(bounds.center, bounds.size))
        }
      }
      clone.userData.objectId = p.id
      clone.userData.objectType = p.type
      const catDef = catalogById.get(p.type)
      if (p.text) {
        if (catDef?.procedural === 'shopSign') {
          // Persistent baked sign face — no hover bubble (so we skip objectText).
          // Reuse a cached (undisposable) text mesh via clone so repeated
          // rebuilds don't leak a fresh CanvasTexture each time.
          clone.add(getSignText(p.type, p.text))
        } else {
          clone.userData.objectText = p.text
        }
      }
      if (catDef?.interaction) {
        clone.userData.objectInteraction = catDef.interaction
        clone.userData.objectInteractOffset = catDef.interactOffset
      }
      if (catDef?.kind) {
        clone.userData.objectKind = catDef.kind
      }
      // Per-instance material clone so the ghost toggle doesn't leak across placements.
      if (catDef?.kind === 'bridge') {
        clone.traverse((o) => {
          if (o instanceof THREE.Mesh && o.material) {
            o.material = (o.material as THREE.Material).clone()
          }
        })
      }
      cloneById.set(p.id, clone)
      group.add(clone)
    }
    // Fresh clones start opaque; the $effect will re-apply ghost next frame
    // if the player is still under a bridge.
    ghostBridgeId = null
  }

  function updatePreview() {
    if (!isEditing() || !previewPos || !selectedType) {
      if (previewGroup) {
        disposeClonedMaterials(previewGroup)
        group.remove(previewGroup)
        previewGroup = null
        previewType = null
      }
      return
    }

    if (previewType !== selectedType) {
      if (previewGroup) {
        disposeClonedMaterials(previewGroup)
        group.remove(previewGroup)
      }
      const template = modelCache.get(selectedType)
      if (!template) {
        getModel(selectedType)
        previewGroup = null
        previewType = null
        return
      }
      previewGroup = template.clone()
      setPreviewMaterial(previewGroup, PREVIEW_OPACITY)
      previewType = selectedType
    }

    if (previewGroup) {
      previewGroup.position.set(previewPos.x, previewPos.y, previewPos.z)
      previewGroup.rotation.y = (rotation * Math.PI) / 180
      if (!previewGroup.parent) {
        group.add(previewGroup)
      }
    }
  }

  $effect(() => {
    void placements
    void selectedId
    void tool
    void isEditorMode
    void currentFloor
    rebuild()
  })

  $effect(() => {
    void previewPos
    void rotation
    void selectedType
    void tool
    void isEditorMode
    updatePreview()
  })

  let ghostBridgeId: number | null = null

  function applyBridgeGhost(placementId: number, ghost: boolean) {
    for (const child of group.children) {
      if (child.userData.objectId !== placementId) continue
      child.traverse((o) => {
        if (!(o instanceof THREE.Mesh)) return
        const m = o.material as THREE.Material
        // Skip collision-only materials baked into a bridge GLB to fill deck
        // holes — they're authored alpha=0 and must stay invisible even when
        // ghost mode ends (otherwise the un-ghost restore turns them into a
        // visible white plane on the deck).
        if (m.name?.startsWith('DeckCollisionInvisible')) return
        m.transparent = ghost
        m.opacity = ghost ? GHOST_OPACITY : 1
        m.depthWrite = !ghost
        // Toggling `transparent` changes blend state — without needsUpdate
        // the shader isn't recompiled and opacity is silently ignored.
        m.needsUpdate = true
        // Draw after the river ribbon (renderOrder=1) so alpha-blended deck
        // sorts above water consistently.
        o.renderOrder = ghost ? 2 : 0
      })
    }
  }

  $effect(() => {
    if (!debugInfo) return
    const id = bridgeManager.findOccludingBridgeId(
      debugInfo.position.x,
      debugInfo.position.y,
      debugInfo.position.z
    )
    if (id === ghostBridgeId) return
    if (ghostBridgeId !== null) applyBridgeGhost(ghostBridgeId, false)
    if (id !== null) applyBridgeGhost(id, true)
    ghostBridgeId = id
  })

  export function getGroup(): THREE.Group {
    return group
  }

  onDestroy(() => {
    for (const child of [...group.children]) {
      disposeClonedMaterials(child)
    }
    group.clear()
    modelCache.clear()
  })
</script>

<T is={group} />

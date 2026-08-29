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
  import type { Position } from '../../network/networkTypes'
  import { playerDebugInfo } from '../../stores/debugStore'
  import type { PlayerDebugInfo } from '../../stores/debugStore'
  import { mapEditorMode } from '../../stores/debugStore'
  import { tileToRegion } from '../../terrain/terrain-constants'
  import { TERRAIN_TILE_SIZE } from '../game-scene/terrain-utils'
  import { objectManager } from '../../managers/objectManager'
  import { bridgeManager } from '../../managers/bridgeManager'
  import { furnitureManager } from '../../managers/furnitureManager'
  import {
    playerVisualFloorLevel,
    playerInsideHouseId,
  } from '../../stores/housingStore'
  import { housingManager } from '../../managers/housingManager'
  import { loadGLB } from '../../utils/gltfCache'
  import {
    CampfireFireParticles,
    TorchFireParticles,
    type FireParticles,
  } from '../../effects/fire-particles'
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
      // take effect immediately. Skip the initial empty-state fire before any
      // region has been loaded.
      const { rx, rz } = lastLoadedRegion
      if (Number.isNaN(rx)) return
      if (catalogById.size > 0) {
        bridgeManager.syncRegion(rx, rz, v.placements, catalogById)
      }
      furnitureManager.syncRegion(rx, rz, v.placements)
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
    playerVisualFloorLevel.subscribe((v) => (currentFloor = v)),
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
    // syncs bridges + furniture for lastLoadedRegion (== rx,rz here).
    currentObjectData.set(data)
    // Sync first, evict after: the new region is in place before its distant
    // neighbours go, so collision is never momentarily absent underfoot.
    furnitureManager.evictDistant(rx, rz)
    bridgeManager.evictDistant(rx, rz)
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
        if (def.kind === 'bridge' && def.bridge) {
          bridgeManager.registerBridgeMesh(objectId, gltf.scene, def.bridge)
          buildGhostMaterials(gltf.scene)
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
      // Plain clones share the cached template's materials — disposing those
      // would force every placement to re-upload textures on the next draw.
      if (child instanceof THREE.Mesh && child.userData.ownsMaterial) {
        ;(child.material as THREE.Material).dispose()
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

  function translucentClone(m: THREE.Material, opacity: number) {
    const t = m.clone()
    t.transparent = true
    t.opacity = opacity
    t.depthWrite = false
    return t
  }

  function setPreviewMaterial(obj: THREE.Object3D, opacity: number) {
    obj.traverse((child) => {
      if (child instanceof THREE.Mesh) {
        child.material = translucentClone(
          child.material as THREE.Material,
          opacity
        )
        child.userData.ownsMaterial = true
      }
    })
  }

  /** objectId → its placement clone in `group`, so the transform-only fast path
   *  can move an existing clone instead of tearing the whole region down. */
  const cloneById = new SvelteMap<number, THREE.Object3D>()
  const isEditing = () => isEditorMode && tool === 'object'

  // Flames of placements whose catalog def has `fire` (hearths). Kept under a
  // separate root: rebuild() sweeps `group`'s children through the material
  // disposer, which must never touch the fire material shared scene-wide.
  const Y_AXIS = new THREE.Vector3(0, 1, 0)
  const fireRoot = new THREE.Group()
  fireRoot.name = 'objectFires'
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const fireSystems = new Map<
    number,
    { system: FireParticles; pos: THREE.Vector3 }
  >()
  /** Currently visible fire systems, refreshed by syncFires(); update() only
   *  ticks these so paused (detached) systems cost nothing per frame. */
  const _activeFires: FireParticles[] = []
  /** World positions of visible flames, refreshed by syncFires(): log-bed
   *  fires (hearths) take the unified light, torches feed the glow pool. */
  const firePositions: THREE.Vector3[] = []
  const torchPositions: THREE.Vector3[] = []
  const _placementPos = new THREE.Vector3()
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const _seenFires = new Set<number>()

  function fireWorldPos(
    p: ObjectPlacement,
    local: Position,
    out: THREE.Vector3
  ) {
    out.set(local.x, local.y, local.z)
    out.applyAxisAngle(Y_AXIS, (p.rotation * Math.PI) / 180)
    return out.add(_placementPos.set(p.x, p.y, p.z))
  }

  /** Sync fire systems to the visible placements. Systems whose placement is
   *  merely filtered out (other floor/house, per `_idShown`) are detached and
   *  paused, not disposed — tearing them down and re-creating them on every
   *  house entry churned garbage and GPU buffers, hitching the entry frame. */
  function syncFires(visible: ObjectPlacement[]) {
    firePositions.length = 0
    torchPositions.length = 0
    _activeFires.length = 0
    _seenFires.clear()
    for (const p of visible) {
      const def = catalogById.get(p.type)
      const fire = def?.fire
      if (!fire) continue
      const isTorch = def?.fireKind === 'torch'
      _seenFires.add(p.id)
      let entry = fireSystems.get(p.id)
      if (!entry) {
        const system = isTorch
          ? new TorchFireParticles()
          : new CampfireFireParticles()
        entry = { system, pos: new THREE.Vector3() }
        fireSystems.set(p.id, entry)
      }
      if (!entry.system.group.parent) fireRoot.add(entry.system.group)
      entry.system.setOrigin(fireWorldPos(p, fire, entry.pos))
      _activeFires.push(entry.system)
      ;(isTorch ? torchPositions : firePositions).push(entry.pos)
    }
    for (const [id, entry] of fireSystems) {
      if (_seenFires.has(id)) continue
      if (_idShown.has(id)) {
        fireRoot.remove(entry.system.group)
        continue
      }
      entry.system.dispose()
      fireSystems.delete(id)
    }
  }

  export function update(deltaTime: number, camera: THREE.Camera | undefined) {
    for (const s of _activeFires) s.update(deltaTime, camera)
  }

  export function getFirePositions(): THREE.Vector3[] {
    return firePositions
  }

  export function getTorchPositions(): THREE.Vector3[] {
    return torchPositions
  }

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

  /** Everything that decides how a placement's clone is built (vs. merely
   *  where it sits). A changed key means teardown + re-clone of that one. */
  function structKeyOf(p: ObjectPlacement): string {
    const sel = isEditing() && p.id === selectedId ? 'S' : ''
    return `${p.type}:${p.text ?? ''}:${sel}`
  }

  function buildClone(
    p: ObjectPlacement,
    template: THREE.Group,
    structKey: string
  ) {
    const clone = template.clone()
    applyPlacementTransform(clone, p)
    if (isEditing() && p.id === selectedId) {
      const bounds = modelBounds.get(p.type)
      if (bounds) clone.add(createSelectionBox(bounds.center, bounds.size))
    }
    clone.userData.objectId = p.id
    clone.userData.objectType = p.type
    clone.userData.structKey = structKey
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
    if (catDef?.kind) clone.userData.objectKind = catDef.kind
    return clone
  }

  /** id → currently shown, over every loaded placement. `has()` answers
   *  "still loaded", `get()` answers "passes the floor/house filter". */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const _idShown = new Map<number, boolean>()

  /** Incremental sync of clones to the loaded placements. Every placement gets
   *  a clone up front; ones filtered out (other floor/house) are detached from
   *  the scene graph but kept in cloneById with their geometry, materials and
   *  compiled GPU state alive — destroy/re-create on house entry churned
   *  garbage and GPU bind groups, hitching the entry frame. Detached clones
   *  cost no render traversal and no raycasts. Teardown only when the
   *  placement is gone or its struct key changes. */
  function rebuild() {
    _idShown.clear()
    for (const p of placements) _idShown.set(p.id, false)
    const visible = visiblePlacements(currentFloor)
    for (const p of visible) _idShown.set(p.id, true)
    syncFires(visible)

    for (const [id, clone] of cloneById) {
      if (_idShown.has(id)) continue
      disposeClonedMaterials(clone)
      group.remove(clone)
      cloneById.delete(id)
    }

    let attached = false
    for (const p of placements) {
      const show = _idShown.get(p.id)
      let clone = cloneById.get(p.id)
      if (clone && show && clone.userData.structKey !== structKeyOf(p)) {
        disposeClonedMaterials(clone)
        group.remove(clone)
        cloneById.delete(p.id)
        clone = undefined
      }
      if (!clone) {
        const template = modelCache.get(p.type)
        if (!template) {
          getModel(p.type)
          continue
        }
        clone = buildClone(p, template, structKeyOf(p))
        cloneById.set(p.id, clone)
      } else if (show) {
        applyPlacementTransform(clone, p)
      }
      if (show) {
        if (!clone.parent) {
          group.add(clone)
          attached = true
        }
      } else if (clone.parent) {
        group.remove(clone)
      }
    }
    // Freshly attached clones start opaque; the $effect will re-apply ghost
    // next frame if the player is still under a bridge.
    if (attached) ghostBridgeId = null
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

  /** Ghost twins of each bridge model's materials, built once per model so
   *  every placement shares the same two material sets and the ghost toggle
   *  is a reference swap — no per-placement clones, no needsUpdate recompiles. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const ghostOf = new Map<THREE.Material, THREE.Material>()
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const solidOf = new Map<THREE.Material, THREE.Material>()

  function buildGhostMaterials(scene: THREE.Object3D) {
    scene.traverse((o) => {
      if (!(o instanceof THREE.Mesh)) return
      const m = o.material as THREE.Material
      // Collision-only materials baked into a bridge GLB to fill deck holes
      // are authored alpha=0 and stay as they are.
      if (ghostOf.has(m) || m.name?.startsWith('DeckCollisionInvisible')) return
      const g = translucentClone(m, GHOST_OPACITY)
      ghostOf.set(m, g)
      solidOf.set(g, m)
    })
  }

  function applyBridgeGhost(placementId: number, ghost: boolean) {
    cloneById.get(placementId)?.traverse((o) => {
      if (!(o instanceof THREE.Mesh)) return
      const swapped = (ghost ? ghostOf : solidOf).get(
        o.material as THREE.Material
      )
      if (!swapped) return
      o.material = swapped
      // Draw after the river ribbon (renderOrder=1) so alpha-blended deck
      // sorts above water consistently.
      o.renderOrder = ghost ? 2 : 0
    })
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
    for (const e of fireSystems.values()) e.system.dispose()
    fireSystems.clear()
    for (const child of [...group.children]) {
      disposeClonedMaterials(child)
    }
    group.clear()
    modelCache.clear()
    for (const g of ghostOf.values()) g.dispose()
    ghostOf.clear()
    solidOf.clear()
  })
</script>

<T is={group} />
<T is={fireRoot} />

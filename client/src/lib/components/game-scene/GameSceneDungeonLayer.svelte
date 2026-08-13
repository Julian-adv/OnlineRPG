<script lang="ts">
  /**
   * GameSceneDungeonLayer — renders the dungeon floor the local player is
   * on. Geometry comes from the shared wasm layout (see dungeonManager);
   * only the current depth is built, rebuilt on depth/dungeon change.
   * Stair shafts are part of both adjacent floors' groups with identical
   * world-space geometry, so the midpoint floor switch is seamless.
   */
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { onDestroy } from 'svelte'
  import { SvelteMap } from 'svelte/reactivity'
  import {
    currentDungeonDepth,
    currentDungeonId,
    dungeonPropsResetRevision,
    dungeonPropsRevision,
  } from '../../stores/dungeonStore'
  import {
    dungeonManager,
    ENTRANCE_DOOR_DEPTH,
    ENTRANCE_DOOR_ID,
    TREASURE_CHEST_PROP_ID,
    type DungeonFloorLayout,
  } from '../../managers/dungeonManager'
  import { objectManager } from '../../managers/objectManager'
  import { loadGLB } from '../../utils/gltfCache'
  import { getObjectModelPath } from '../../utils/modelPaths'
  import { rotatedRectAabb } from '../../utils/objectFootprint'
  import type { DoorLeaf, InteriorDoor } from '../../utils/dungeon-geometry'
  import { networkManager } from '../../network/socket'
  import {
    buildDungeonEntranceGroup,
    buildDungeonFloorGroup,
    disposeDungeonGroup,
    UP_SHAFT_GROUP_NAME,
    type WallRun,
  } from '../../utils/dungeon-geometry'
  import { getGhostHousingMaterial } from '../../utils/housing-textures'
  import { isoCameraOccludesPlayer } from '../../utils/iso-occlusion'
  import { passabilityDebugVisible } from '../../stores/debugStore'
  import { pushPassabilityEdges } from '../../utils/passability-wireframe'

  interface Props {
    /** Fired the frame the player comes into range of a clicked barrel/crate,
     *  handing off to the player swing that breaks it at the contact frame. */
    onPropReady?: (
      entranceId: string,
      depth: number,
      propId: number,
      x: number,
      z: number
    ) => void
  }
  let { onPropReady }: Props = $props()

  /** Once the player walking up to a clicked prop is within this range, the
   *  break/open is requested. Kept inside the server's 2.5m so a borderline
   *  float position never gets rejected. */
  const PROP_INTERACT_TRIGGER_RANGE = 2.0
  /** Catalog id of the broken variant rendered when a prop is destroyed. */
  const BROKEN_VARIANT: Record<string, string> = {
    barrel: 'barrel_broken',
    crate: 'crate_pieces',
  }
  const isBreakable = (kind: string) => kind in BROKEN_VARIANT
  /** Dungeon chest props render from this catalog id (the animated GLB, not the
   *  static `chest.glb`) so a click can play the lid-open clip. */
  const CHEST_ANIMATED_ID = 'chest_animated'
  const CHEST_OPEN_CLIP = 'ChestOpen'
  /** How far the open lid's rear corner reaches behind the chest origin (along
   *  local −Z): the lid swings up and ~105° back over the hinge. Measured from
   *  chest_animated.glb. The chest is pushed this far (less the half-cell it
   *  already sits in) off its back wall so the opening lid never clips it. */
  const CHEST_LID_BACK_REACH = 0.8
  /** Gap kept between the fully-open lid and the back wall. */
  const CHEST_LID_WALL_GAP = 0.06
  /** Yaw (deg) that maps the model's back (local −Z) onto the chosen back wall. */
  const CHEST_BACK_WALL_YAW = { N: 0, S: 180, W: 90, E: 270 } as const

  /** One placed prop, tracked so a live break can swap just it (not rebuild the
   *  whole floor). `cellX/cellZ` locate the broken debris at the cell center. */
  interface PropEntry {
    clones: THREE.Object3D[]
    kind: string
    propId: number
    cellX: number
    cellZ: number
    rotationDeg: number
    broken: boolean
    /** Chest only: whether its lid-open animation has been started. */
    opened?: boolean
  }
  let propEntries = new Map<number, PropEntry>()
  /** Flame world-positions of the current floor's wall torches. Read every frame
   *  by the players layer (via a provider prop) to drive the wall-torch light
   *  pool + the unified shadow light. Swapped on floor rebuild, cleared on exit. */
  let wallTorchPositions: THREE.Vector3[] = []
  /** The chest lid-open clip, captured once from the animated chest GLB and
   *  shared by every chest clone (each gets its own AnimationMixer). */
  let chestOpenClip: THREE.AnimationClip | null = null
  /** One-shot prop mixers (the chest lid-open clip), ticked each frame. The
   *  clip is LoopOnce + clampWhenFinished, so it holds its final pose once
   *  done. Cleared with the props on a floor rebuild. The spilled coin pile is
   *  no longer rendered here — it's a pickable ground item (see groundItem). */
  let propMixers: THREE.AnimationMixer[] = []

  const root = new THREE.Group()
  let currentGroup: THREE.Group | null = null
  let entranceGroup: THREE.Group | null = null
  /** Decorative room clutter (barrel/crate/chest GLBs) for the current floor,
   *  kept in its own group on `root` — never inside currentGroup, whose
   *  disposer would otherwise dispose these shared cached GLB resources.
   *  Stable reference (created once, children swapped per floor) so the array
   *  handed to click raycasting via getPropMeshes never goes stale. */
  const propsGroup = new THREE.Group()
  root.add(propsGroup)
  /** Per-kind metrics measured once per GLB: base-seat offset (−bbox.min.y),
   *  height, and horizontal half-extents (hx/hz) — used to seat the model, to
   *  align a chest's long side to its wall, and to inset a boxy prop off it. */
  const propMetrics = new SvelteMap<
    string,
    { seatY: number; height: number; hx: number; hz: number }
  >()
  /** Nest a stacked prop slightly into the one below to hide the seam. */
  const PROP_STACK_NEST = 0.97
  /** Wall torch: base height up the (3m) wall — flame sits ~2.3m. */
  const TORCH_MOUNT_Y = 1.7
  /** Wall torch: how far the flame sits above the mount base (so the light is
   *  placed at the flame, not the bracket). */
  const TORCH_FLAME_RISE = 0.6
  /** Wall torch: how far the flame reaches off the wall into the room (along the
   *  model's local +Z), so the light source clears the wall surface. */
  const TORCH_FLAME_REACH = 0.25
  /** Wall torch: how far the mount's back face seats *into* the wall from its
   *  room-facing boundary. 0 = back face flush on the wall surface. */
  const TORCH_WALL_INSET = 0
  /** A crate is left crooked when its random yaw lands more than this many
   *  degrees off a right angle (≈ a quarter of crates). */
  const CRATE_ASKEW_THRESH = 34
  /** Scales that off-axis amount down to a believable lean (~19–25°). */
  const CRATE_ASKEW_SCALE = 0.55
  /** Double entrance doors; swing open/shut when clicked (house-door style). */
  let entranceDoors: DoorLeaf[] = []
  /** Eased open fraction (0 shut → 1 fully open), lerped per frame toward the
   *  entrance door's synced open/shut state (door key 0/0). */
  let doorOpenAmount = 0
  /** Whether the surface entrance is currently shown (depth 0). Tracked so we
   *  can reconcile the door visual to the store on the hidden→shown edge. */
  let entranceVisible = false
  let builtKey = ''
  let entranceKey = ''
  let lastPropsResetRevision = 0

  // ── Up-shaft occlusion fade ──────────────────────────────
  // The staircase you arrive by occludes the player from the iso camera when
  // they walk behind it; fade it to a ghost material (like trees/houses).
  interface GhostMesh {
    mesh: THREE.Mesh
    base: THREE.Material
    ghost: THREE.Material
  }
  let upShaftMeshes: GhostMesh[] = []
  let upShaftAABB: THREE.Box3 | null = null
  let upShaftOccluded = false
  /** Ray inside the AABB before it counts as occluding (matches housing). */
  const MIN_OCCLUSION_DEPTH = 0.3

  // ── Wall-run occlusion fade ──────────────────────────────
  // Any wall run (all four sides) that ends up between the iso camera and the
  // player is ghosted, per-run (per-run AABB, so the others stay solid). The
  // runs are thin (0.1m), so the SW camera ray only ever crosses ~0.1 of one — a
  // much smaller occlusion depth than the bulky up-shaft AABB.
  interface WallRunFade {
    mesh: THREE.Mesh
    base: THREE.Material
    ghost: THREE.Material
    aabb: THREE.Box3
  }
  let wallRuns: WallRunFade[] = []
  const WALL_RUN_MIN_OCCLUSION = 0.05

  // ── Interior room doors ──────────────────────────────────
  // Double doors across corridor mouths in room walls. Click to open/close
  // (server-synced, like the entrance door); a shut door seals its corridor
  // mouth in the wasm passability cells. The leaves live in their own pickable
  // group (not the floor click group), positioned at the floor origin and
  // replaced on every floor (re)build.
  let interiorDoors: InteriorDoor[] = []
  const interiorDoorGroup = new THREE.Group()
  root.add(interiorDoorGroup)

  function clearGroup() {
    if (currentGroup) {
      root.remove(currentGroup)
      disposeDungeonGroup(currentGroup)
      currentGroup = null
    }
    clearProps()
    upShaftMeshes = []
    upShaftAABB = null
    upShaftOccluded = false
    wallRuns = []
    // Door leaves live in this persistent sibling group, so disposeDungeonGroup
    // (which only walks currentGroup) won't reach them — dispose here. Materials
    // are shared (never disposed), like disposeDungeonGroup.
    for (const c of [...interiorDoorGroup.children]) {
      c.traverse((o) => {
        if (o instanceof THREE.Mesh) o.geometry.dispose()
      })
      interiorDoorGroup.remove(c)
    }
    interiorDoors = []
  }

  /** Detach the current floor's props. Their GLB geometry/materials are shared
   *  via the module-level gltfCache, so (unlike disposeDungeonGroup) we only
   *  drop the clones and let GC reclaim them — disposing would corrupt the
   *  cache and every other instance. */
  function clearProps() {
    for (const c of [...propsGroup.children]) propsGroup.remove(c)
    propEntries = new Map()
    wallTorchPositions = []
    for (const m of propMixers) m.stopAllAction()
    propMixers = []
  }

  /** Measure (and cache) a model's seat offset, height and half-extents. */
  function measureProp(kind: string, template: THREE.Object3D) {
    let m = propMetrics.get(kind)
    if (!m) {
      template.updateMatrixWorld(true)
      const box = new THREE.Box3().setFromObject(template)
      const size = box.getSize(new THREE.Vector3())
      m = {
        seatY: -box.min.y,
        height: Math.max(0.1, size.y),
        hx: size.x / 2,
        hz: size.z / 2,
      }
      propMetrics.set(kind, m)
    }
    return m
  }

  /** Strip shadows on, raycast off for a decorative (non-interactive) clone. */
  function tagDecorative(clone: THREE.Object3D) {
    clone.traverse((o) => {
      if (o instanceof THREE.Mesh) {
        o.castShadow = true
        o.receiveShadow = true
        o.raycast = () => {} // never intercept click-to-move
      }
    })
  }

  /** Tag a clone clickable for the prop raycast pass (break/open on walk-up).
   *  Read by inputHandler's prop pass via the userData keys. */
  function tagInteractiveProp(
    clone: THREE.Object3D,
    propId: number,
    kind: string,
    depth: number,
    opts: { breakable?: boolean; openable?: boolean }
  ) {
    clone.userData.dungeonProp = true
    clone.userData.propId = propId
    clone.userData.propKind = kind
    clone.userData.propDepth = depth
    clone.userData.propEntranceId = dungeonManager.dungeonId
    if (opts.breakable) clone.userData.propBreakable = true
    if (opts.openable) clone.userData.propOpenable = true
  }

  /** Resolve a catalog id to its loaded GLB template, or null on any failure.
   *  Latches the shared lid-open clip the first time the animated chest loads. */
  async function loadPropTemplate(
    catalogId: string
  ): Promise<THREE.Object3D | null> {
    const def = objectManager.getCatalogEntry(catalogId)
    if (!def?.model) return null
    try {
      const gltf = await loadGLB(getObjectModelPath(def.model))
      if (
        catalogId === CHEST_ANIMATED_ID &&
        !chestOpenClip &&
        gltf.animations.length
      ) {
        chestOpenClip =
          gltf.animations.find((a) => a.name === CHEST_OPEN_CLIP) ??
          gltf.animations[0]
      }
      return gltf.scene
    } catch {
      return null
    }
  }

  /**
   * Build one normal (un-broken) prop into `group`/`entries`. Barrels and crates
   * are tagged interactive (clickable to break); chests stay decorative. The
   * per-kind yaw + cell-inset tuning is unchanged from the original placement.
   */
  async function addNormalProp(
    group: THREE.Group,
    entries: Map<number, PropEntry>,
    index: number,
    prop: DungeonFloorLayout['props'][number],
    key: string,
    depth: number,
    carvedAt: (x: number, z: number) => boolean,
    torchPositions: THREE.Vector3[]
  ) {
    // Chests render from the animated GLB (lid rigged for the open clip); the
    // other props use their own catalog model.
    const template = await loadPropTemplate(
      prop.kind === 'chest' ? CHEST_ANIMATED_ID : prop.kind
    )
    if (!template) return
    if (key !== builtKey) return // floor changed mid-load — abandon
    const m = measureProp(prop.kind, template)

    // Wall torch: hangs high on a room's north/east wall, decorative only. The
    // model's wall side is its local −Z=0 face (the bracket back) with the torch
    // body reaching into the room along +Z and the base at local y=0. The
    // generator hands us the room-facing yaw (north wall → 0°, east wall → 270°);
    // we seat the back face flush against that wall and raise it up the wall.
    if (prop.kind === 'torch_wall') {
      const clone = template.clone()
      const yawDeg = prop.rotation
      clone.rotation.y = (yawDeg * Math.PI) / 180
      let px = prop.x + 0.5
      let pz = prop.z + 0.5
      // Push the back face onto the mounted wall. The generator only ever emits
      // a north (0°) or east (270°) facing — one per room.
      if (yawDeg === 0)
        pz = prop.z - TORCH_WALL_INSET // faces +Z, wall on −Z (N)
      else if (yawDeg === 270) px = prop.x + 1 + TORCH_WALL_INSET // wall on +X (E)
      clone.position.set(px, TORCH_MOUNT_Y, pz)
      tagDecorative(clone)
      group.add(clone)
      // Record the flame's world position for the players-layer light pool. The
      // flame rises off the bracket and reaches off the wall along the model's
      // local +Z (here baked into the clone yaw), so push the light out there.
      const yaw = clone.rotation.y
      torchPositions.push(
        new THREE.Vector3(
          dungeonManager.originX + px + Math.sin(yaw) * TORCH_FLAME_REACH,
          dungeonManager.floorY(depth) + TORCH_MOUNT_Y + TORCH_FLAME_RISE,
          dungeonManager.originZ + pz + Math.cos(yaw) * TORCH_FLAME_REACH
        )
      )
      entries.set(index, {
        clones: [clone],
        kind: prop.kind,
        propId: index,
        cellX: prop.x,
        cellZ: prop.z,
        rotationDeg: prop.rotation,
        broken: false,
        opened: false,
      })
      return
    }

    // Chests sit with their hinge (model back, local −Z) against a wall and
    // their opening (local +Z) facing into the room. The lid swings up and back
    // over the hinge, so the chest is also pushed off that wall (below) to
    // clear it. Pick the back wall from the carved grid, preferring a Z-facing
    // wall so the long (X) side runs parallel to it.
    let chestYawDeg = 0
    let chestBackWall: 'N' | 'S' | 'W' | 'E' | null = null
    if (prop.kind === 'chest') {
      if (!carvedAt(prop.x, prop.z - 1))
        chestBackWall = 'N' // wall on −Z
      else if (!carvedAt(prop.x, prop.z + 1))
        chestBackWall = 'S' // wall on +Z
      else if (!carvedAt(prop.x - 1, prop.z))
        chestBackWall = 'W' // wall on −X
      else if (!carvedAt(prop.x + 1, prop.z)) chestBackWall = 'E' // wall on +X
      // Map the model's back (local −Z) onto the chosen wall.
      chestYawDeg = chestBackWall ? CHEST_BACK_WALL_YAW[chestBackWall] : 0
    }

    const breakable = isBreakable(prop.kind)
    const clones: THREE.Object3D[] = []
    const count = Math.max(1, prop.stack)
    for (let i = 0; i < count; i++) {
      const clone = template.clone()

      // Yaw per kind:
      //  • chest — opening faces the room, hinge to the wall (computed above).
      //  • crate — square box: snap to 90° so it sits flush, but leave the
      //    occasional one crooked; quarter-turn each stacked tier.
      //  • barrel — cylindrical, footprint is rotation-invariant: free yaw +
      //    small per-tier twist.
      let yawDeg: number
      if (prop.kind === 'chest') {
        yawDeg = chestYawDeg
      } else if (prop.kind === 'crate') {
        const quarter = Math.round(prop.rotation / 90)
        const dev = prop.rotation - quarter * 90 // −45..45, uniform
        const tiltDeg =
          Math.abs(dev) > CRATE_ASKEW_THRESH ? dev * CRATE_ASKEW_SCALE : 0
        yawDeg = quarter * 90 + tiltDeg + i * 90
      } else {
        yawDeg = prop.rotation + i * 23
      }

      // Place within the cell. Chests get pushed off their back wall far enough
      // that the open lid clears it, with the long ends tucked off any
      // perpendicular (corner) wall. Crates push off any wall by their rotated
      // footprint overhang so they stay inside the cell; barrels are round and
      // small, so they stay centered.
      let px = prop.x + 0.5
      let pz = prop.z + 0.5
      // Barrels are round and small, so they stay centered; chests and crates
      // need their rotated footprint kept inside the cell.
      if (prop.kind !== 'barrel') {
        const aabb = rotatedRectAabb(
          -m.hx,
          m.hx,
          -m.hz,
          m.hz,
          (yawDeg * Math.PI) / 180
        )
        const insetX = Math.max(0, aabb.maxX - 0.5)
        const insetZ = Math.max(0, aabb.maxZ - 0.5)
        if (prop.kind === 'chest') {
          // Push off the back wall so the open lid clears it; tuck the long ends
          // off any perpendicular (corner) wall on the other axis.
          const backPush = Math.max(
            0,
            CHEST_LID_BACK_REACH + CHEST_LID_WALL_GAP - 0.5
          )
          if (chestBackWall === 'N' || chestBackWall === 'S') {
            pz += chestBackWall === 'N' ? backPush : -backPush
            if (!carvedAt(prop.x - 1, prop.z)) px += insetX
            else if (!carvedAt(prop.x + 1, prop.z)) px -= insetX
          } else if (chestBackWall === 'W' || chestBackWall === 'E') {
            px += chestBackWall === 'W' ? backPush : -backPush
            if (!carvedAt(prop.x, prop.z - 1)) pz += insetZ
            else if (!carvedAt(prop.x, prop.z + 1)) pz -= insetZ
          }
        } else {
          // Crate: push off any bordering wall by the footprint overhang.
          if (!carvedAt(prop.x - 1, prop.z)) px += insetX
          else if (!carvedAt(prop.x + 1, prop.z)) px -= insetX
          if (!carvedAt(prop.x, prop.z - 1)) pz += insetZ
          else if (!carvedAt(prop.x, prop.z + 1)) pz -= insetZ
        }
      }

      clone.position.set(px, m.seatY + i * m.height * PROP_STACK_NEST, pz)
      clone.rotation.y = (yawDeg * Math.PI) / 180
      // Interactive props (clicked → walk up → break/open). Barrels/crates
      // break; chests open.
      const openable = prop.kind === 'chest'
      const interactive = breakable || openable
      if (interactive) {
        tagInteractiveProp(clone, index, prop.kind, depth, {
          breakable,
          openable,
        })
      }
      clone.traverse((o) => {
        if (o instanceof THREE.Mesh) {
          o.castShadow = true
          o.receiveShadow = true
          if (!interactive) o.raycast = () => {} // decorative: never intercept
        }
      })
      group.add(clone)
      clones.push(clone)
    }
    entries.set(index, {
      clones,
      kind: prop.kind,
      propId: index,
      cellX: prop.x,
      cellZ: prop.z,
      rotationDeg: prop.rotation,
      broken: false,
      opened: false,
    })
  }

  /** Place the final floor's treasure chest into `group`/`entries` under its
   *  sentinel id: the same animated chest GLB as the clutter chests, seated on
   *  the chest cell, facing south (+Z) so the open lid shows the inside,
   *  and clickable to open. The lid plays on the DungeonChestOpened broadcast;
   *  the loot the server ejects lands as pickable ground items around it. */
  async function addTreasureChest(
    group: THREE.Group,
    entries: Map<number, PropEntry>,
    chest: [number, number],
    key: string,
    depth: number
  ) {
    const template = await loadPropTemplate(CHEST_ANIMATED_ID)
    if (!template) return
    if (key !== builtKey) return
    const m = measureProp('chest', template)
    const clone = template.clone()
    clone.position.set(chest[0] + 0.5, m.seatY, chest[1] + 0.5)
    clone.rotation.y = 0
    tagInteractiveProp(clone, TREASURE_CHEST_PROP_ID, 'chest', depth, {
      openable: true,
    })
    clone.traverse((o) => {
      if (o instanceof THREE.Mesh) {
        o.castShadow = true
        o.receiveShadow = true
      }
    })
    group.add(clone)
    entries.set(TREASURE_CHEST_PROP_ID, {
      clones: [clone],
      kind: 'chest',
      propId: TREASURE_CHEST_PROP_ID,
      cellX: chest[0],
      cellZ: chest[1],
      rotationDeg: 0,
      broken: false,
      opened: false,
    })
  }

  /** Load + seat a kind's broken-debris variant (single, cell-centered) clone.
   *  Returns null if the variant is missing, the GLB fails to load, or the floor
   *  changed mid-load. Shared by the initial build and the live swap. */
  async function buildBrokenClone(
    kind: string,
    cellX: number,
    cellZ: number,
    rotationDeg: number,
    key: string
  ): Promise<THREE.Object3D | null> {
    const variantId = BROKEN_VARIANT[kind]
    const template = variantId ? await loadPropTemplate(variantId) : null
    if (!template) return null
    if (key !== builtKey) return null
    const m = measureProp(variantId, template)
    const clone = template.clone()
    clone.position.set(cellX + 0.5, m.seatY, cellZ + 0.5)
    clone.rotation.y = (rotationDeg * Math.PI) / 180
    tagDecorative(clone)
    return clone
  }

  /** Place a prop's debris variant into `group`/`entries` (already broken at load). */
  async function addBrokenProp(
    group: THREE.Group,
    entries: Map<number, PropEntry>,
    index: number,
    prop: DungeonFloorLayout['props'][number],
    key: string
  ) {
    const clone = await buildBrokenClone(
      prop.kind,
      prop.x,
      prop.z,
      prop.rotation,
      key
    )
    if (!clone) return
    group.add(clone)
    entries.set(index, {
      clones: [clone],
      kind: prop.kind,
      propId: index,
      cellX: prop.x,
      cellZ: prop.z,
      rotationDeg: prop.rotation,
      broken: true,
    })
  }

  /**
   * Async-load and place the floor's decorative props. The dungeon-extras GLBs
   * are authored at world scale, XZ-centered with their base at Y=0, so they
   * only need seating + the cell-center offset. Props already broken (per the
   * server snapshot) render as debris directly. Committed only if the floor is
   * still current — depth/dungeon can change across the awaits; `key` is the
   * builtKey snapshot taken when the build started.
   */
  async function buildProps(
    layout: DungeonFloorLayout,
    key: string,
    depth: number
  ) {
    const specs = layout.props ?? []
    if (specs.length === 0 && !layout.chest) return
    await objectManager.fetchCatalog()
    if (key !== builtKey) return

    const group = new THREE.Group()
    group.position.set(
      dungeonManager.originX,
      dungeonManager.floorY(depth),
      dungeonManager.originZ
    )
    const entries = new Map<number, PropEntry>()
    const torchPositions: THREE.Vector3[] = []

    const grid = dungeonManager.consts.grid
    const carvedAt = (x: number, z: number) =>
      x >= 0 && x < grid && z >= 0 && z < grid && layout.carved[x + z * grid]
    const broken = dungeonManager.brokenPropsForDepth(depth)

    for (let index = 0; index < specs.length; index++) {
      const prop = specs[index]
      if (broken.has(index) && isBreakable(prop.kind)) {
        await addBrokenProp(group, entries, index, prop, key)
      } else {
        await addNormalProp(
          group,
          entries,
          index,
          prop,
          key,
          depth,
          carvedAt,
          torchPositions
        )
      }
      if (key !== builtKey) return // floor changed mid-load — abandon
    }

    if (layout.chest) {
      await addTreasureChest(group, entries, layout.chest, key, depth)
    }

    if (key !== builtKey) return
    clearProps()
    // Move the freshly built clones into the stable props group, matching its
    // offset to the temp group's so world positions are preserved.
    propsGroup.position.copy(group.position)
    while (group.children.length) propsGroup.add(group.children[0])
    propEntries = entries
    wallTorchPositions = torchPositions
    // Catch any breaks/opens that landed while the GLBs were loading. Chests
    // already open at build time snap to the open pose (no entrance swing).
    reconcileBrokenProps(depth, key)
    reconcileOpenedProps(depth, key, true)
    reconcileTreasureChest(true)
  }

  /** Swap every prop the server says is broken but that's still rendered whole. */
  function reconcileBrokenProps(depth: number, key: string) {
    if (key !== builtKey) return
    const broken = dungeonManager.brokenPropsForDepth(depth)
    for (const index of broken) {
      const entry = propEntries.get(index)
      if (entry && !entry.broken) void swapPropToBroken(index, key)
    }
  }

  /** Replace one prop's whole-model clones with its broken debris variant. */
  async function swapPropToBroken(index: number, key: string) {
    const entry = propEntries.get(index)
    if (!entry || entry.broken || !isBreakable(entry.kind)) return
    entry.broken = true // claim before the await so a re-run won't double-swap
    const clone = await buildBrokenClone(
      entry.kind,
      entry.cellX,
      entry.cellZ,
      entry.rotationDeg,
      key
    )
    if (!clone) return
    for (const c of entry.clones) propsGroup.remove(c)
    propsGroup.add(clone)
    entry.clones = [clone]
  }

  /** Play the lid-open animation on every chest the server says is open but
   *  that's still rendered shut. `instant` jumps straight to the open pose (for
   *  chests already open when the floor builds); otherwise the lid animates. */
  function reconcileOpenedProps(depth: number, key: string, instant: boolean) {
    if (key !== builtKey) return
    const opened = dungeonManager.openedPropsForDepth(depth)
    for (const index of opened) {
      const entry = propEntries.get(index)
      if (entry && entry.kind === 'chest' && !entry.opened) {
        openChest(entry, instant)
      }
    }
  }

  /** Play (or snap) the treasure chest's lid-open pose once this session has
   *  seen its DungeonChestOpened broadcast. */
  function reconcileTreasureChest(instant: boolean) {
    const entry = propEntries.get(TREASURE_CHEST_PROP_ID)
    if (entry && dungeonManager.treasureChestOpened) openChest(entry, instant)
  }

  /** Start (or snap to the end of) a chest's lid-open animation. Each chest gets
   *  its own mixer bound to its clone; the shared clip animates the `chest_lid`
   *  node, so cloning by name resolves correctly. The coins the chest spills are
   *  spawned server-side as a pickable ground item, rendered by the ground-item
   *  layer — not here. */
  function openChest(entry: PropEntry, instant: boolean) {
    if (entry.opened) return
    const clone = entry.clones[0]
    if (!clone) return
    entry.opened = true
    if (chestOpenClip) {
      const mixer = new THREE.AnimationMixer(clone)
      const action = mixer.clipAction(chestOpenClip)
      action.loop = THREE.LoopOnce
      action.clampWhenFinished = true // hold the lid open after the clip ends
      action.play()
      if (instant) mixer.setTime(chestOpenClip.duration) // already-open: skip the swing
      propMixers.push(mixer)
    }
  }

  /** Cache the up-shaft sub-group's meshes + world AABB for the fade pass.
   *  `localAABB` is the group-local box from buildDungeonFloorGroup. */
  function cacheUpShaft(group: THREE.Group, localAABB: THREE.Box3) {
    const upGroup = group.getObjectByName(UP_SHAFT_GROUP_NAME)
    if (!upGroup) return
    upShaftAABB = localAABB.clone().translate(group.position)
    upGroup.traverse((obj) => {
      if (!(obj instanceof THREE.Mesh)) return
      const idx = obj.userData.textureIndex as number
      upShaftMeshes.push({
        mesh: obj,
        base: obj.material as THREE.Material,
        ghost: getGhostHousingMaterial(idx),
      })
    })
  }

  /** Cache each wall run's mesh, ghost material and world AABB for the per-run
   *  fade pass. `group.position` is set before this is called. */
  function cacheWallRuns(group: THREE.Group, runs: WallRun[]) {
    for (const r of runs) {
      const idx = r.mesh.userData.textureIndex as number
      wallRuns.push({
        mesh: r.mesh,
        base: r.mesh.material as THREE.Material,
        ghost: getGhostHousingMaterial(idx),
        aabb: r.localAABB.clone().translate(group.position),
      })
    }
  }

  function clearEntranceGroup() {
    if (entranceGroup) {
      root.remove(entranceGroup)
      disposeDungeonGroup(entranceGroup)
      entranceGroup = null
      entranceDoors = []
      doorOpenAmount = 0
      entranceVisible = false
    }
  }

  /** Write the current eased open fraction (doorOpenAmount) to both entrance
   *  door leaves' hinge rotation. Shared by the per-frame swing and the
   *  snap-to-store on (re)appear. */
  function applyDoorRotation() {
    for (const leaf of entranceDoors)
      leaf.pivot.rotation.y =
        leaf.closedAngle + (leaf.openAngle - leaf.closedAngle) * doorOpenAmount
  }

  // ── Passability debug overlay (red wireframe on blocked cell edges) ──
  // Shares the `/passability` toggle with the housing overlay so one
  // command shows blocked edges everywhere. Draws the floor whose shaft the
  // player is on (the entry shaft lives in floor 1, hence Math.max(1, depth)).
  const debugPassGroup = new THREE.Group()
  debugPassGroup.name = 'dungeonPassabilityDebug'
  debugPassGroup.visible = false
  root.add(debugPassGroup)
  const debugLineMaterial = new THREE.LineBasicMaterial({ color: 0xff0000 })

  function clearDebugPass() {
    while (debugPassGroup.children.length > 0) {
      const child = debugPassGroup.children[0]
      debugPassGroup.remove(child)
      if (child instanceof THREE.LineSegments) child.geometry.dispose()
    }
  }

  function rebuildDebugPass() {
    clearDebugPass()
    if (!debugPassGroup.visible || !dungeonManager.active) return
    const floorLevel = dungeonManager.passabilityFloor(
      Math.max(1, $currentDungeonDepth)
    )
    const f = dungeonManager.floorPassabilityCells(floorLevel)
    if (!f) return

    const verts: number[] = []
    pushPassabilityEdges(
      verts,
      f.cells,
      f.width,
      f.depth,
      f.originX,
      f.originZ,
      f.yBase
    )
    if (verts.length > 0) {
      const geo = new THREE.BufferGeometry()
      geo.setAttribute('position', new THREE.Float32BufferAttribute(verts, 3))
      const lines = new THREE.LineSegments(geo, debugLineMaterial)
      lines.frustumCulled = false
      debugPassGroup.add(lines)
    }
  }

  // Toggle + re-draw on dungeon/depth change. rebuildDebugPass early-outs
  // when hidden, so this is free while the overlay is off.
  $effect(() => {
    debugPassGroup.visible = $passabilityDebugVisible
    void $currentDungeonId
    void $currentDungeonDepth
    rebuildDebugPass()
  })

  // Surface entrance structure (descending stairs + pit walls). The geometry
  // depends only on the dungeon id, so it's built once per dungeon and only
  // its visibility tracks depth: shown at depth 0, hidden underground where
  // the floor group owns the shaft (rendering both would z-fight).
  $effect(() => {
    const id = $currentDungeonId
    const depth = $currentDungeonDepth
    if ((id ?? '') !== entranceKey) {
      entranceKey = id ?? ''
      clearEntranceGroup()
      if (id) {
        const first = dungeonManager.layoutAt(1)
        if (first) {
          const c = dungeonManager.consts
          const built = buildDungeonEntranceGroup(first.upShaft, {
            grid: c.grid,
            wallHeight: c.wallHeight,
            floorHeight: c.floorHeight,
            shaftW: c.shaftW,
            shaftLen: c.shaftLen,
          })
          entranceGroup = built.group
          entranceDoors = built.doors
          doorOpenAmount = 0
          for (const leaf of built.doors)
            leaf.pivot.rotation.y = leaf.closedAngle
          entranceGroup.position.set(
            dungeonManager.originX,
            dungeonManager.entrancePos!.y,
            dungeonManager.originZ
          )
          root.add(entranceGroup)
        }
        // Pull the current open/closed state of every door in this dungeon
        // (entrance + interior, all depths) so doors others left open render
        // correctly. Live toggles arrive via DungeonDoorToggled broadcasts.
        networkManager.sendRequestDungeonDoors(id)
      }
    }
    if (entranceGroup) {
      const showEntrance = depth === 0
      if (showEntrance && !entranceVisible) {
        // The surface entrance just (re)appeared — returning to the surface
        // after a respawn/teleport reuses this group without a rebuild. Snap
        // the door visual to the live open/shut state so a stale open angle
        // can't linger while collision (entranceBlocksMovement) treats it as
        // shut, which would look open but block the player from descending.
        doorOpenAmount = dungeonManager.isDoorOpen(
          ENTRANCE_DOOR_DEPTH,
          ENTRANCE_DOOR_ID
        )
          ? 1
          : 0
        applyDoorRotation()
      }
      entranceVisible = showEntrance
      entranceGroup.visible = showEntrance
    }
  })

  $effect(() => {
    const id = $currentDungeonId
    const depth = $currentDungeonDepth
    const key = id && depth >= 1 ? `${id}:${depth}` : ''
    if (key === builtKey) return
    builtKey = key
    clearGroup()
    if (!key) return

    const layout = dungeonManager.layoutAt(depth)
    if (!layout) return
    const c = dungeonManager.consts
    const built = buildDungeonFloorGroup(
      layout,
      {
        grid: c.grid,
        wallHeight: c.wallHeight,
        floorHeight: c.floorHeight,
        shaftW: c.shaftW,
        shaftLen: c.shaftLen,
      },
      dungeonManager.interiorDoorsAt(depth)
    )
    currentGroup = built.group
    currentGroup.position.set(
      dungeonManager.originX,
      dungeonManager.floorY(depth),
      dungeonManager.originZ
    )
    root.add(currentGroup)
    cacheUpShaft(currentGroup, built.upShaftAABB)
    cacheWallRuns(currentGroup, built.wallRuns)

    // Interior doors: parent the leaves to the door group (same origin as the
    // floor); collision comes from the wasm passability cells.
    interiorDoors = built.doors
    interiorDoorGroup.position.copy(currentGroup.position)
    for (const door of built.doors)
      for (const leaf of door.leaves) interiorDoorGroup.add(leaf.pivot)
    void buildProps(layout, key, depth)
  })

  // Reconcile prop meshes with the server's broken set whenever it changes
  // (live break broadcast, or the on-entry snapshot arriving after the build).
  $effect(() => {
    void $dungeonPropsRevision
    reconcileBrokenProps($currentDungeonDepth, builtKey)
    reconcileOpenedProps($currentDungeonDepth, builtKey, false)
    reconcileTreasureChest(false)
  })

  // Debug reset (or any backwards authoritative snapshot): rebuild the current
  // floor's props so broken debris and open chest poses return to their normal
  // models.
  $effect(() => {
    const resetRevision = $dungeonPropsResetRevision
    if (resetRevision === lastPropsResetRevision) return
    lastPropsResetRevision = resetRevision
    if (!builtKey) return
    const depth = $currentDungeonDepth
    const layout = dungeonManager.layoutAt(depth)
    if (!layout) return
    void buildProps(layout, builtKey, depth)
  })

  onDestroy(() => {
    clearGroup()
    clearEntranceGroup()
    clearDebugPass()
    debugLineMaterial.dispose()
  })

  /** True once the player is close enough to a pending prop on the current
   *  floor to fire its break/open. */
  function pendingPropReached(
    pending: { depth: number; x: number; z: number },
    depth: number,
    playerX: number,
    playerZ: number
  ): boolean {
    if (depth !== pending.depth) return false
    const dx = playerX - pending.x
    const dz = playerZ - pending.z
    return (
      dx * dx + dz * dz <=
      PROP_INTERACT_TRIGGER_RANGE * PROP_INTERACT_TRIGGER_RANGE
    )
  }

  /** Per-frame: stair-shaft floor transitions + pending prop walk-ups.
   *  `deltaMs` advances the chest lid-open animation. */
  export function update(
    playerX: number,
    playerY: number,
    playerZ: number,
    deltaMs = 0
  ) {
    dungeonManager.updateFromPlayerPosition(playerX, playerZ)

    const doorResyncId = dungeonManager.takeDoorSnapshotRequest()
    if (doorResyncId) networkManager.sendRequestDungeonDoors(doorResyncId)

    // Advance one-shot GLB clips; clamped actions hold their final poses.
    if (propMixers.length > 0) {
      const dt = deltaMs / 1000
      for (const m of propMixers) m.update(dt)
    }

    // (The entrance roof has no proximity hide — it's always shown at depth 0.)

    // Fade the up-shaft stairs to a ghost when they occlude the player.
    if (upShaftAABB && upShaftMeshes.length > 0) {
      const occ = isoCameraOccludesPlayer(
        upShaftAABB,
        playerX,
        playerY,
        playerZ,
        MIN_OCCLUSION_DEPTH
      )
      if (occ !== upShaftOccluded) {
        upShaftOccluded = occ
        for (const m of upShaftMeshes) {
          m.mesh.material = occ ? m.ghost : m.base
        }
      }
    }

    // Fade each wall run that occludes the player to a ghost. The mesh's current
    // material is the single source of truth for its occluded state.
    for (const w of wallRuns) {
      const occ = isoCameraOccludesPlayer(
        w.aabb,
        playerX,
        playerY,
        playerZ,
        WALL_RUN_MIN_OCCLUSION
      )
      if (occ !== (w.mesh.material === w.ghost)) {
        w.mesh.material = occ ? w.ghost : w.base
      }
    }

    // Swing both door leaves toward their click-toggled, server-synced target
    // (~0.35s either way at 60fps). Driven by the door state, not proximity.
    if (entranceDoors.length > 0) {
      const target = dungeonManager.isDoorOpen(
        ENTRANCE_DOOR_DEPTH,
        ENTRANCE_DOOR_ID
      )
        ? 1
        : 0
      doorOpenAmount += (target - doorOpenAmount) * 0.12
      applyDoorRotation()
    }

    // Interior room doors swing toward their server-synced open/shut state
    // (set by click toggles, same ease as the entrance door). Settled doors
    // (the common case) snap and skip, so we don't re-write rotations forever.
    for (const door of interiorDoors) {
      const target = dungeonManager.isDoorOpen(door.depth, door.doorId) ? 1 : 0
      if (door.open === target) continue
      door.open += (target - door.open) * 0.12
      if (Math.abs(target - door.open) < 1e-3) door.open = target
      for (const leaf of door.leaves)
        leaf.pivot.rotation.y =
          leaf.closedAngle + (leaf.openAngle - leaf.closedAngle) * door.open
    }

    if (!dungeonManager.active) return
    const depth = $currentDungeonDepth

    // Pending prop break: the player walked up to a barrel/crate they clicked —
    // request the break once within range. The server validates and broadcasts;
    // the visual swap happens on receipt (handles other players' breaks too).
    const pending = dungeonManager.pendingBreak
    if (pending && pendingPropReached(pending, depth, playerX, playerZ)) {
      // Reached the prop — hand off to the player swing, which breaks it at the
      // contact frame. Clear pending first so this fires once.
      const id = dungeonManager.dungeonId!
      const { depth: d, propId, x, z } = pending
      dungeonManager.clearPendingBreak()
      onPropReady?.(id, d, propId, x, z)
    }

    // Pending chest open: the player walked up to a chest they clicked — request
    // the open once within range. The server validates and broadcasts; the lid
    // animation plays on receipt (handles other players' opens too). The
    // final-floor treasure chest shares this walk-up under its sentinel id.
    const pendingOpen = dungeonManager.pendingOpen
    if (
      pendingOpen &&
      pendingPropReached(pendingOpen, depth, playerX, playerZ)
    ) {
      const id = dungeonManager.dungeonId!
      const { depth: d, propId } = pendingOpen
      dungeonManager.clearPendingOpen()
      if (propId === TREASURE_CHEST_PROP_ID) {
        networkManager.sendOpenDungeonChest(id)
      } else {
        networkManager.sendOpenDungeonProp(id, d, propId)
      }
    }
  }

  export function getGroup(): THREE.Group {
    return root
  }

  /** Flame world-positions of the current floor's wall torches, for the players
   *  layer's wall-torch light pool + unified shadow light. Returns the live array
   *  (swapped on rebuild, emptied on exit), so callers must read it per frame
   *  rather than caching it. */
  export function getWallTorchPositions(): THREE.Vector3[] {
    return wallTorchPositions
  }

  /**
   * The active floor's group — the click-to-move ground raycast target while
   * underground. Returns ONLY the current floor (its walkable slab + up/down
   * shaft stairs; the wall runs inside it are already non-pickable), NOT the
   * whole dungeon `root`. The root's sibling groups — the surface entrance
   * shell (hidden underground but, since THREE's raycaster does not skip
   * invisible objects, still pickable; its walls/roof sit well above the floor),
   * its swing doors, decorative props and debug overlays — would otherwise
   * intercept a click meant for the floor near the stairs and resolve it to a
   * bogus cell up at the entrance, sending the player back up. Touches the
   * dungeon stores so the binding refreshes on enter/exit and floor change.
   */
  export function getFloorGroup(): THREE.Group | null {
    void $currentDungeonId
    void $currentDungeonDepth
    return currentGroup
  }

  /** Raycast targets for clicking breakable props (barrels/crates). Only the
   *  breakable clones have raycast enabled, so chests/debris won't intercept. */
  export function getPropMeshes(): THREE.Object3D[] {
    return [propsGroup]
  }

  /**
   * Click raycast targets for the dungeon doors: the entrance doors at depth 0
   * (where the entrance is shown), and the interior room doors underground.
   * Reads the dungeon stores so the GameScene prop re-evaluates on enter/exit
   * and depth change.
   */
  export function getDoorMeshes(): THREE.Object3D[] {
    void $currentDungeonId
    if ($currentDungeonDepth === 0)
      return entranceDoors.map((leaf) => leaf.pivot)
    return interiorDoors.flatMap((door) => door.leaves.map((l) => l.pivot))
  }
</script>

<T is={root} />

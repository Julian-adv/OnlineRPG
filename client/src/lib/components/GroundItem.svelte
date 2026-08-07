<script lang="ts">
  import { T } from '@threlte/core'
  import { onDestroy } from 'svelte'
  import * as THREE from 'three'
  import { getItemDef } from '../data/itemDefs'
  import { getWeaponModelPath } from '../utils/modelPaths'
  import { loadGLB } from '../utils/gltfCache'
  import { loadIconTexture } from '../utils/iconTextureCache'
  import { createRng } from '../utils/simplex-noise'
  import { localPlayerRightHand } from '../stores/playerHandRegistry'
  import type { TerrainHeightManager } from '../managers/terrainHeightManager'
  import { entityGroundY } from '../managers/entity-ground'
  import {
    evaluateSpawnAnimation,
    type GroundItemData,
  } from '../managers/groundItemManager'

  interface Props {
    data: GroundItemData
    animationTimeMs?: number
    heightManager?: TerrainHeightManager
    heightRevision?: number
  }

  let {
    data,
    animationTimeMs = 0,
    heightManager,
    heightRevision = 0,
  }: Props = $props()

  const def = $derived(getItemDef(data.itemDefId))
  const label = $derived(def?.name ?? data.itemDefId)
  const UP = new THREE.Vector3(0, 1, 0)
  const TERRAIN_NORMAL_SAMPLE_DISTANCE = 0.75
  const MAX_TERRAIN_Y_DELTA_FOR_TILT = 0.75

  // Twinkling sparkles that rise off the item in place of the old edge glow.
  // The count scales with the model's size so a big item (e.g. a spear) emits
  // from more points than a small potion.
  const SPARK_DENSITY_PER_M = 34 // sparks per metre of bounding-box diagonal
  const SPARK_COUNT_MIN = 10
  const SPARK_COUNT_MAX = 44
  const SPARK_LIFETIME = 1.6 // seconds for one rise cycle
  const SPARK_RISE = 0.45 // metres a spark climbs over its lifetime
  const SPARK_DRIFT = 0.06 // max horizontal wander
  const SPARK_SIZE = 0.07 // sprite world size at full twinkle
  const SPARK_TWINKLE_FREQ = 9 // brightness flicker speed

  let worldModelScene: THREE.Object3D | undefined = $state()
  // Local-space (relative to origin) bounding box of the loaded model. Single
  // source of truth for everything derived from the model's footprint/volume:
  // the pad's center and radius, and the spark emission points.
  let worldModelBox = $state<{
    min: { x: number; y: number; z: number }
    max: { x: number; y: number; z: number }
  } | null>(null)
  let groundParentRef: THREE.Group | undefined = $state()
  let terrainAlignedRef: THREE.Group | undefined = $state()

  // Self-animating loot (the dungeon coin pile): the GLB ships a spill/settle
  // clip that plays once on spawn, so the pile pours out of the chest and lands
  // instead of using the generic loot arc. The mixer is advanced by real frame
  // deltas (LoopOnce + clampWhenFinished holds the settled pose) — seeking with
  // setTime each frame would snap a finished, paused action back to frame 0.
  // `selfAnimated` flips on once the clip is set up; `poured` flips on when it
  // finishes (the glow waits for that so it outlines the settled pile, not the
  // mid-air coins).
  let selfMixer: THREE.AnimationMixer | undefined
  let selfClipLastMs = 0
  let selfAnimated = $state(false)
  let poured = $state(false)

  function cloneScene(
    scene: THREE.Object3D,
    onMesh: (mesh: THREE.Mesh) => void
  ): THREE.Object3D {
    const clone = scene.clone(true)
    clone.traverse((child) => {
      if (child instanceof THREE.Mesh) onMesh(child)
    })
    return clone
  }

  function cloneGroundItemScene(scene: THREE.Object3D): THREE.Object3D {
    return cloneScene(scene, (mesh) => {
      mesh.castShadow = true
      mesh.receiveShadow = true
    })
  }

  // Soft radial sparkle, generated once and shared by every spark sprite.
  function makeSparkTexture(): THREE.CanvasTexture {
    const c = document.createElement('canvas')
    c.width = 64
    c.height = 64
    const ctx = c.getContext('2d')!
    const g = ctx.createRadialGradient(32, 32, 0, 32, 32, 32)
    g.addColorStop(0, 'rgba(255,255,255,1)')
    g.addColorStop(0.25, 'rgba(255,241,189,0.85)')
    g.addColorStop(1, 'rgba(255,221,130,0)')
    ctx.fillStyle = g
    ctx.fillRect(0, 0, 64, 64)
    return new THREE.CanvasTexture(c)
  }
  const sparkTexture = makeSparkTexture()

  /** Bind a clip to a freshly cloned scene as a one-shot (LoopOnce + clamp) and
   *  return the mixer. The clip is node-based TRS (no skeleton), so the clone's
   *  named nodes bind directly. Pass `holdAtEnd` to jump straight to the settled
   *  pose (used for the glow outline, which we don't animate frame-by-frame). */
  function bindClipOnce(
    scene: THREE.Object3D,
    clip: THREE.AnimationClip,
    holdAtEnd: boolean
  ): THREE.AnimationMixer {
    const mixer = new THREE.AnimationMixer(scene)
    const action = mixer.clipAction(clip)
    action.loop = THREE.LoopOnce
    action.clampWhenFinished = true
    action.play()
    if (holdAtEnd) mixer.setTime(clip.duration)
    return mixer
  }

  function getTerrainAlignmentQuaternion(
    worldX: number,
    worldY: number,
    worldZ: number,
    shouldTilt: boolean
  ): THREE.Quaternion {
    if (!shouldTilt || !heightManager?.hasHeightData(worldX, worldZ)) {
      return new THREE.Quaternion()
    }

    const d = TERRAIN_NORMAL_SAMPLE_DISTANCE
    if (
      !heightManager.hasHeightData(worldX - d, worldZ) ||
      !heightManager.hasHeightData(worldX + d, worldZ) ||
      !heightManager.hasHeightData(worldX, worldZ - d) ||
      !heightManager.hasHeightData(worldX, worldZ + d)
    ) {
      return new THREE.Quaternion()
    }

    const terrainY = heightManager.getHeightAtWorldPosition(worldX, worldZ)
    if (Math.abs(worldY - terrainY) > MAX_TERRAIN_Y_DELTA_FOR_TILT) {
      return new THREE.Quaternion()
    }

    const hL = heightManager.getHeightAtWorldPosition(worldX - d, worldZ)
    const hR = heightManager.getHeightAtWorldPosition(worldX + d, worldZ)
    const hB = heightManager.getHeightAtWorldPosition(worldX, worldZ - d)
    const hF = heightManager.getHeightAtWorldPosition(worldX, worldZ + d)
    const normal = new THREE.Vector3(hL - hR, 2 * d, hB - hF).normalize()
    return new THREE.Quaternion().setFromUnitVectors(UP, normal)
  }

  $effect(() => {
    const worldModel = def?.worldModel
    if (!worldModel) {
      worldModelScene = undefined
      worldModelBox = null
      return
    }
    let cancelled = false
    let loadedScene: THREE.Object3D | undefined
    selfMixer = undefined
    selfClipLastMs = 0
    selfAnimated = false
    poured = false
    const path = getWeaponModelPath(worldModel)
    loadGLB(path).then((gltf) => {
      if (cancelled) return
      const scene = cloneGroundItemScene(gltf.scene)
      // An animated loot model (e.g. the coin pile) plays its spill/settle clip
      // once on spawn: the model is ticked frame-by-frame (see the mixer effect)
      // and only becomes interactable once the pour finishes (`poured`, set by
      // the mixer's own `finished` event). Static models have no clip.
      const clip = gltf.animations[0]
      if (clip) {
        selfMixer = bindClipOnce(scene, clip, false)
        selfMixer.addEventListener('finished', () => {
          poured = true
        })
        selfAnimated = true
      }
      // Measure the footprint/volume from the pose things actually sit under. An
      // animated pile is still at frame 0 (pre-spill, tiny) on `scene`, so for
      // the measurement use a throwaway clone jumped to the settled end pose.
      let measureSource = scene
      if (clip) {
        measureSource = cloneGroundItemScene(gltf.scene)
        bindClipOnce(measureSource, clip, true)
      }
      const box = new THREE.Box3().setFromObject(measureSource)
      worldModelBox = {
        min: { x: box.min.x, y: box.min.y, z: box.min.z },
        max: { x: box.max.x, y: box.max.y, z: box.max.z },
      }
      loadedScene = scene
      worldModelScene = scene
    })
    return () => {
      cancelled = true
      selfMixer?.stopAllAction()
      selfMixer = undefined
      if (loadedScene?.parent) loadedScene.parent.remove(loadedScene)
    }
  })

  // Tick the coin pile's spill clip only while it's still pouring. Gating on the
  // reactive `selfAnimated`/`poured` before reading the frame clock means Svelte
  // never schedules this for static loot, and stops re-running it once a pile
  // settles (the `finished` event flips `poured`, which clamps the final pose).
  $effect(() => {
    if (!selfAnimated || poured) return
    const now = animationTimeMs
    if (!selfMixer) return
    if (selfClipLastMs === 0) selfClipLastMs = now
    const dt = (now - selfClipLastMs) / 1000
    selfClipLastMs = now
    if (dt > 0) selfMixer.update(dt)
  })

  $effect(() => {
    const scene = worldModelScene
    const ground = groundParentRef
    if (!scene || !ground) return
    // A spread-out coin pile held up to the face reads as an awkward flat
    // slab, so self-animating loot is never parented to the hand; the root
    // group's inHand visibility gate hides it at the grab instead.
    if (selfAnimated) {
      if (scene.parent !== ground) {
        scene.position.set(0, 0, 0)
        scene.rotation.set(0, 0, 0)
        ground.add(scene)
      }
      return
    }
    const hand = data.inHand ? $localPlayerRightHand : null
    const targetParent = hand ?? ground
    if (scene.parent === targetParent) return
    scene.position.set(0, hand ? 0.08 : 0, 0)
    scene.rotation.set(0, 0, 0)
    targetParent.add(scene)
  })

  function makeNameTexture(text: string): THREE.CanvasTexture {
    const c = document.createElement('canvas')
    c.width = 256
    c.height = 64
    const ctx = c.getContext('2d')!
    ctx.fillStyle = 'rgba(0,0,0,0.6)'
    ctx.fillRect(0, 0, 256, 64)
    ctx.font = 'bold 28px Courier New'
    ctx.fillStyle = '#f0c040'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillText(text, 128, 32)
    return new THREE.CanvasTexture(c)
  }

  const nameTexture = $derived(
    def?.worldModel || def?.icon ? null : makeNameTexture(label)
  )

  // Unit count for a pile of stackables — one model is drawn either way, so the
  // badge is the only thing saying twelve potions lie here rather than one.
  // Outlined text on a canvas cut to fit the glyphs, the way TextLabel does it:
  // a fixed-size canvas would waste most of its pixels on a string this short
  // and leave the badge blurry once scaled down to item size.
  const QTY_FONT_PX = 64
  const QTY_PIXELS_PER_UNIT = 320
  const QTY_BADGE_LIFT = 0.12

  function makeQuantityTexture(text: string): {
    texture: THREE.CanvasTexture
    width: number
    height: number
  } {
    const font = `bold ${QTY_FONT_PX}px sans-serif`
    const outline = QTY_FONT_PX * 0.16
    const pad = Math.ceil(outline)
    const measureCtx = document.createElement('canvas').getContext('2d')!
    measureCtx.font = font

    const c = document.createElement('canvas')
    c.width = Math.ceil(measureCtx.measureText(text).width) + pad * 2
    c.height = Math.ceil(QTY_FONT_PX * 1.25) + pad * 2
    const ctx = c.getContext('2d')!
    ctx.font = font
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.lineJoin = 'round'
    ctx.strokeStyle = 'rgba(0,0,0,0.85)'
    ctx.lineWidth = outline
    ctx.strokeText(text, c.width / 2, c.height / 2)
    ctx.fillStyle = '#ffe9a8'
    ctx.fillText(text, c.width / 2, c.height / 2)

    const texture = new THREE.CanvasTexture(c)
    texture.minFilter = THREE.LinearFilter
    texture.magFilter = THREE.LinearFilter
    texture.colorSpace = THREE.SRGBColorSpace
    return {
      texture,
      width: c.width / QTY_PIXELS_PER_UNIT,
      height: c.height / QTY_PIXELS_PER_UNIT,
    }
  }

  const qtyBadge = $derived(
    data.quantity > 1 ? makeQuantityTexture(`x${data.quantity}`) : null
  )
  // Sits above the model's top, so a spear's badge clears the spear.
  const qtyBadgeY = $derived(
    (worldModelBox ? worldModelBox.max.y : 0.3) + QTY_BADGE_LIFT
  )
  $effect(() => {
    const badge = qtyBadge
    return () => badge?.texture.dispose()
  })

  // Icon billboard for items with no world model (fish, jewellery, armor):
  // the inventory icon floats over the spot instead of a placeholder box.
  // Textures come from the shared icon cache — do not dispose them.
  const ICON_SPRITE_SIZE = 0.55
  const ICON_SHADOW_OFFSET = 0.05
  const ICON_SHADOW_OPACITY = 0.85
  let iconTexture = $state<THREE.Texture | null>(null)
  $effect(() => {
    const icon = !def?.worldModel && def?.icon ? def.icon : null
    if (!icon) {
      iconTexture = null
      return
    }
    let cancelled = false
    loadIconTexture(`/items/${icon}`).then((tex) => {
      if (!cancelled) iconTexture = tex
    })
    return () => {
      cancelled = true
      iconTexture = null
    }
  })

  // Representation tier: 3D model when loaded, else the inventory icon
  // billboard, else the placeholder box (also shown while either loads).
  const displayMode = $derived(
    worldModelScene ? 'model' : iconTexture ? 'icon' : 'box'
  )

  onDestroy(() => {
    nameTexture?.dispose()
    sparkTexture.dispose()
  })

  // Self-animating loot pours in via its own clip, so it skips the generic loot
  // arc and rests near floor level (the clip bakes its own rise/fall) instead of
  // the usual +0.3 hover. The settled coin pile's lowest coins sink ~0.036m
  // below the model origin, so a small lift keeps them from clipping under the
  // floor. Outdoor terrain (non-negative floor) gets a touch more lift than a
  // dungeon's flat floor (negative floor level) to keep the wide pile from
  // clipping into small ground rises.
  const SELF_ANIM_REST_HOVER_TERRAIN = 0.03
  const SELF_ANIM_REST_HOVER_DUNGEON = 0.01
  const spawnTransform = $derived(
    data.spawnAnimation && !data.inHand && !selfAnimated
      ? evaluateSpawnAnimation(data.spawnAnimation, animationTimeMs)
      : null
  )
  const spinZ = $derived(spawnTransform?.spinZ ?? 0)
  // Idle spin for the placeholder box only; models rest and sprites billboard.
  const BOX_SPIN_SPEED = 1.5
  const boxSpinY = $derived(
    displayMode === 'box' && !data.spawnAnimation
      ? (animationTimeMs / 1000) * BOX_SPIN_SPEED
      : 0
  )
  // Items rendered as a 3D world model are authored to sit on their base
  // (origin at the model's bottom), so they rest just above the ground with a
  // small lift to avoid z-fighting and clipping into minor terrain rises. The
  // larger +0.3 hover is only for the icon-billboard fallback, which floats
  // above the spot so the flat sprite reads clearly.
  const WORLD_MODEL_REST_HOVER = 0.05
  const restHover = $derived(
    selfAnimated
      ? data.floorLevel < 0
        ? SELF_ANIM_REST_HOVER_DUNGEON
        : SELF_ANIM_REST_HOVER_TERRAIN
      : worldModelScene
        ? WORLD_MODEL_REST_HOVER
        : 0.3
  )
  const displayX = $derived(data.position.x + (spawnTransform?.offsetX ?? 0))
  const displayZ = $derived(data.position.z + (spawnTransform?.offsetZ ?? 0))
  const baseY = $derived.by(() => {
    void heightRevision
    return data.inHand
      ? data.position.y
      : entityGroundY(
          heightManager ?? null,
          data.floorLevel,
          data.position.x,
          data.position.z,
          data.position.y,
          data.position.y
        )
  })
  const displayY = $derived(baseY + restHover + (spawnTransform?.offsetY ?? 0))
  const shouldTiltToTerrain = $derived(!data.inHand && !spawnTransform)
  // Flat pickup pad laid under a grounded item so small models still offer a
  // generous click target. It is a child of the root group (so a click on it
  // walks up to `groundItemId`), but counter-offsets the root's hover/spawn-arc
  // lift so it stays planted on the ground with a hair of clearance.
  const GROUND_PAD_RADIUS = 0.32
  const GROUND_PAD_CLEARANCE = 0.012
  // Pad sits under the model's visual center and circumscribes its footprint
  // (radius = half the x/z diagonal); both fall back to defaults until a model
  // box is known. Derived from the box so there is one source of truth.
  const worldModelCenter = $derived(
    worldModelBox
      ? {
          x: (worldModelBox.min.x + worldModelBox.max.x) / 2,
          z: (worldModelBox.min.z + worldModelBox.max.z) / 2,
        }
      : { x: 0, z: 0 }
  )
  const groundPadRadius = $derived(
    worldModelBox
      ? Math.hypot(
          worldModelBox.max.x - worldModelBox.min.x,
          worldModelBox.max.z - worldModelBox.min.z
        ) / 2
      : GROUND_PAD_RADIUS
  )
  const groundPadY = $derived(
    GROUND_PAD_CLEARANCE - restHover - (spawnTransform?.offsetY ?? 0)
  )
  // The pad lives at the root (flat, unspun), but the model is yawed by its
  // resting rotation, so spin the local bbox center by the same yaw to keep the
  // pad under the model's visual center.
  const groundPadOffset = $derived(
    new THREE.Vector3(worldModelCenter.x, 0, worldModelCenter.z).applyAxisAngle(
      UP,
      data.restingRotationY
    )
  )
  // Depends only on the (post-animation, constant) display position and tilt
  // flag — so a resting item computes its terrain alignment once and stops,
  // rather than re-running terrain height lookups every frame.
  const terrainAlignmentQuaternion = $derived(
    getTerrainAlignmentQuaternion(
      displayX,
      baseY,
      displayZ,
      shouldTiltToTerrain
    )
  )
  // Show the pad and sparkles once a self-animating pile has settled, so they
  // sit under/around the resting coins rather than the ones still pouring
  // through the air.
  const showPad = $derived(!data.inHand && (!selfAnimated || poured))

  // Per-spark emission seeds: a fixed point sampled across the model's box
  // (yawed to match the rendered model), a phase offset so they don't pulse in
  // unison, and horizontal drift. Deterministic per item, recomputed only when
  // the model box loads — not every frame.
  const sparkSeeds = $derived.by(() => {
    const box = worldModelBox
    // Icon-billboard items (no worldModel) have no volume to emit from, so they
    // intentionally get no sparkles.
    if (!box) return []
    // Seeded per item so each spark keeps a stable identity across frames.
    const rand = createRng((data.instanceId * 2654435761) >>> 0)
    const yaw = data.restingRotationY
    const cos = Math.cos(yaw)
    const sin = Math.sin(yaw)
    const diagonal = Math.hypot(
      box.max.x - box.min.x,
      box.max.y - box.min.y,
      box.max.z - box.min.z
    )
    const count = Math.max(
      SPARK_COUNT_MIN,
      Math.min(SPARK_COUNT_MAX, Math.round(diagonal * SPARK_DENSITY_PER_M))
    )
    return Array.from({ length: count }, () => {
      // Sample within the box, biased toward the centre so sparks hug the item.
      const lx = box.min.x + (box.max.x - box.min.x) * (0.2 + 0.6 * rand())
      const ly = box.min.y + (box.max.y - box.min.y) * (0.1 + 0.8 * rand())
      const lz = box.min.z + (box.max.z - box.min.z) * (0.2 + 0.6 * rand())
      const driftAngle = rand() * Math.PI * 2
      return {
        ox: lx * cos + lz * sin,
        oy: ly,
        oz: -lx * sin + lz * cos,
        dx: Math.cos(driftAngle) * SPARK_DRIFT,
        dz: Math.sin(driftAngle) * SPARK_DRIFT,
        phase: rand(),
        twinkle: rand() * Math.PI * 2,
      }
    })
  })

  // Current per-frame position/scale/opacity for each spark, driven by the
  // shared animation clock. Each rises over its lifetime, fading in then out
  // while flickering, and loops back to its emission point.
  const sparkStates = $derived.by(() => {
    const t = animationTimeMs / 1000
    return sparkSeeds.map((s) => {
      const phase = (t / SPARK_LIFETIME + s.phase) % 1
      const fade = Math.sin(phase * Math.PI) // 0 at ends, 1 mid-rise
      const twinkle = 0.5 + 0.5 * Math.sin(t * SPARK_TWINKLE_FREQ + s.twinkle)
      return {
        x: s.ox + s.dx * phase,
        y: s.oy + phase * SPARK_RISE,
        z: s.oz + s.dz * phase,
        opacity: fade * (0.35 + 0.65 * twinkle),
        scale: SPARK_SIZE * (0.55 + 0.45 * twinkle) * (0.5 + 0.5 * fade),
      }
    })
  })

  $effect(() => {
    terrainAlignedRef?.quaternion.copy(terrainAlignmentQuaternion)
  })
</script>

<!-- The whole subtree hides the moment the pickup grabs the item (inHand);
     a hand-held model is reparented out of it. Raycasts ignore `visible`, so
     the pickup pad additionally unmounts via showPad. -->
<T.Group
  position.x={displayX}
  position.y={displayY}
  position.z={displayZ}
  visible={!data.inHand}
  userData={{ groundItemId: data.instanceId }}
>
  {#if showPad}
    <!-- Invisible pickup pad: enlarges the click target for small grounded
         items without drawing anything. Fully transparent but still raycast
         (visible=true), so a click on it walks up to `groundItemId`. Kept
         outside the tilt/spin groups so it lies flat and still. -->
    <T.Mesh
      rotation.x={-Math.PI / 2}
      position.x={groundPadOffset.x}
      position.y={groundPadY}
      position.z={groundPadOffset.z}
      renderOrder={0}
    >
      <T.CircleGeometry args={[groundPadRadius, 40]} />
      <T.MeshBasicMaterial transparent={true} opacity={0} depthWrite={false} />
    </T.Mesh>
  {/if}

  {#if showPad}
    <!-- Rising sparkles: emitted from points across the item, climbing and
         twinkling. Kept at the root group so they rise along world up rather
         than tilting with the terrain or spinning with the model. -->
    {#each sparkStates as spark, i (i)}
      <T.Sprite
        position.x={spark.x}
        position.y={spark.y}
        position.z={spark.z}
        scale={[spark.scale, spark.scale, spark.scale]}
        renderOrder={2}
      >
        <T.SpriteMaterial
          map={sparkTexture}
          color="#ffe7a8"
          transparent={true}
          opacity={spark.opacity}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
        />
      </T.Sprite>
    {/each}
  {/if}

  {#if qtyBadge && !data.inHand}
    <!-- Count badge for a pile. At the root group so it billboards upright
         instead of tilting with the terrain or spinning with the model. -->
    <T.Sprite
      position.y={qtyBadgeY}
      scale={[qtyBadge.width, qtyBadge.height, 1]}
      renderOrder={3}
    >
      <T.SpriteMaterial
        map={qtyBadge.texture}
        transparent={true}
        depthWrite={false}
      />
    </T.Sprite>
  {/if}

  {#if displayMode === 'icon'}
    <!-- Icon + black-tinted shadow copy. Kept at the root so the shadow's
         offset doesn't orbit with the spin group; sprites ignore parent
         rotation, so the spawn-arc spin goes through material rotation. -->
    <T.Sprite
      position.x={ICON_SHADOW_OFFSET}
      position.y={-ICON_SHADOW_OFFSET}
      scale={[ICON_SPRITE_SIZE, ICON_SPRITE_SIZE, 1]}
      renderOrder={0}
    >
      <T.SpriteMaterial
        map={iconTexture}
        color="#000000"
        transparent={true}
        opacity={ICON_SHADOW_OPACITY}
        depthWrite={false}
        rotation={spinZ}
      />
    </T.Sprite>
    <T.Sprite scale={[ICON_SPRITE_SIZE, ICON_SPRITE_SIZE, 1]} renderOrder={1}>
      <T.SpriteMaterial map={iconTexture} transparent={true} rotation={spinZ} />
    </T.Sprite>
  {/if}

  <T.Group bind:ref={terrainAlignedRef}>
    <T.Group rotation.y={data.restingRotationY + boxSpinY} rotation.z={spinZ}>
      <T.Group bind:ref={groundParentRef} />

      {#if displayMode === 'box'}
        <T.Mesh>
          <T.BoxGeometry args={[0.3, 0.3, 0.3]} />
          <T.MeshStandardMaterial color="#f0c040" />
        </T.Mesh>

        {#if nameTexture}
          <T.Sprite position.y={0.5} scale={[label.length * 0.08, 0.2, 1]}>
            <T.SpriteMaterial map={nameTexture} transparent={true} />
          </T.Sprite>
        {/if}
      {/if}
    </T.Group>
  </T.Group>
</T.Group>

<script lang="ts">
  import { gameStore } from '../stores/gameStore'
  import { worldMapVisible } from '../stores/debugStore'
  import { minimapVersion } from '../stores/editorStore'
  import { calendarShown } from '../stores/inventoryStore'
  import {
    currentDungeonDepth,
    discoveredDungeonIds,
  } from '../stores/dungeonStore'
  import { playerVisualFloorLevel } from '../stores/housingStore'
  import { houseMapFootprints } from '../stores/housingMapStore'
  import { DUNGEON_ENTRANCES } from '../data/dungeonDefs'
  import { RegionImageCache } from '../terrain/regionImageCache'
  import { pickMinimapSourceSize } from '../terrain/regionMinimapGenerator'
  import {
    graphicsQuality,
    getEffectivePreset,
  } from '../stores/graphicsSettings'
  import { REGION_CELLS, TILE_DIM } from '../terrain/terrain-constants'
  import {
    wrapWorldX,
    WORLD_MIN_REGION_Z,
    WORLD_MAX_REGION_Z,
  } from '../terrain/world-wrap'
  import {
    MAP_ROTATE_ANGLE,
    drawDungeonEntranceMarkers,
    drawHouseMapFootprints,
    headingToMapAngle,
  } from '../utils/map-structures'
  import SelfMarker from './SelfMarker.svelte'

  /** Canvas size in CSS pixels. */
  const SIZE = 180
  /** World meters shown across the widget — fixed zoom, sized so the region
   *  bakes (1 px/m) only ever downscale. */
  const VIEW_WORLD = 384
  /** Player movement below this step doesn't trigger a redraw. */
  const REDRAW_STEP_M = 2
  /** ~4 regions cover the rotated view; keep a little slack for movement. */
  const IMAGE_CACHE_LIMIT = 12

  const graphicsPreset = $derived(getEffectivePreset($graphicsQuality))

  let canvasEl = $state<HTMLCanvasElement>()

  /** The bakes are surface terrain: underground or upstairs there is nothing
   *  truthful to draw, so the widget hides rather than showing the surface. */
  let onSurface = $derived(
    $currentDungeonDepth === 0 && $playerVisualFloorLevel === 0
  )

  const regionImages = new RegionImageCache()
  regionImages.limit = IMAGE_CACHE_LIMIT

  // Quantized player position: redraw on ~2 m moves, not per frame.
  // Wrapped like the world map keeps its view center on the baked range.
  let playerX = $derived(wrapWorldX($gameStore.currentPlayer?.position.x ?? 0))
  let playerZ = $derived($gameStore.currentPlayer?.position.z ?? 0)
  let qx = $derived(Math.round(playerX / REDRAW_STEP_M))
  let qz = $derived(Math.round(playerZ / REDRAW_STEP_M))
  let markerAngle = $derived(
    headingToMapAngle($gameStore.currentPlayer?.rotation ?? 0)
  )

  let renderGeneration = 0

  $effect(() => {
    if (!canvasEl) return

    // Reactive triggers: quantized position, regenerated bakes.
    const px = qx * REDRAW_STEP_M
    const pz = qz * REDRAW_STEP_M
    const ver = $minimapVersion
    const houses = $houseMapFootprints
    const knownDungeons = $discoveredDungeonIds
    const gen = ++renderGeneration

    const dpr = Math.min(
      window.devicePixelRatio || 1,
      graphicsPreset.pixelRatioCap
    )
    const backingSize = Math.round(SIZE * dpr)
    if (canvasEl.width !== backingSize) canvasEl.width = backingSize
    if (canvasEl.height !== backingSize) canvasEl.height = backingSize
    const ctx = canvasEl.getContext('2d')!
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.imageSmoothingEnabled = true
    ctx.imageSmoothingQuality = 'high'

    const scale = SIZE / VIEW_WORLD
    const projectedRegionPx = REGION_CELLS * scale * dpr
    const sourceSize = pickMinimapSourceSize(projectedRegionPx, 512)
    const viewLeft = px - VIEW_WORLD / 2
    const viewTop = pz - VIEW_WORLD / 2

    ctx.fillStyle = '#000'
    ctx.fillRect(0, 0, SIZE, SIZE)

    // Rotated square needs sqrt(2) coverage, same as the world map.
    const expanded = VIEW_WORLD * Math.SQRT2
    const expLeft = px - expanded / 2
    const expTop = pz - expanded / 2
    const minRx = Math.floor((expLeft + TILE_DIM / 2) / REGION_CELLS)
    const maxRx = Math.floor((expLeft + expanded + TILE_DIM / 2) / REGION_CELLS)
    const minRz = Math.floor((expTop + TILE_DIM / 2) / REGION_CELLS)
    const maxRz = Math.floor((expTop + expanded + TILE_DIM / 2) / REGION_CELLS)

    const rotated = (draw: () => void) => {
      ctx.save()
      ctx.translate(SIZE / 2, SIZE / 2)
      ctx.rotate(MAP_ROTATE_ANGLE)
      ctx.translate(-SIZE / 2, -SIZE / 2)
      draw()
      ctx.restore()
    }

    const promises: Promise<void>[] = []
    for (let rz = minRz; rz <= maxRz; rz++) {
      if (rz < WORLD_MIN_REGION_Z || rz > WORLD_MAX_REGION_Z) continue
      for (let rx = minRx; rx <= maxRx; rx++) {
        const regionWorldX = rx * REGION_CELLS - TILE_DIM / 2
        const regionWorldZ = rz * REGION_CELLS - TILE_DIM / 2
        const drawX = Math.floor((regionWorldX - viewLeft) * scale)
        const drawY = Math.floor((regionWorldZ - viewTop) * scale)
        const drawSize = Math.ceil(REGION_CELLS * scale)
        promises.push(
          regionImages.load(rx, rz, ver, sourceSize).then((img) => {
            if (gen !== renderGeneration || !img) return
            rotated(() => ctx.drawImage(img, drawX, drawY, drawSize, drawSize))
          })
        )
      }
    }

    Promise.all(promises).then(() => {
      if (gen !== renderGeneration) return

      const transform = {
        centerX: px,
        viewLeft,
        viewTop,
        scale,
      }
      rotated(() => {
        drawHouseMapFootprints(ctx, houses, transform)
        drawDungeonEntranceMarkers(
          ctx,
          DUNGEON_ENTRANCES.filter((entrance) =>
            knownDungeons.has(entrance.id)
          ),
          transform
        )
      })
    })
  })
</script>

{#if onSurface}
  <button
    class="minimap"
    class:lowered={$calendarShown}
    title="Open world map (M)"
    aria-label="Open world map"
    onclick={() => worldMapVisible.set(true)}
  >
    <canvas bind:this={canvasEl} style="width: {SIZE}px; height: {SIZE}px;"
    ></canvas>
    <SelfMarker left={SIZE / 2} top={SIZE / 2} angle={markerAngle} size={16} />
  </button>
{/if}

<style>
  .minimap {
    position: absolute;
    /* Just below the time widget (top: 9px + 36px tall). */
    top: 53px;
    right: 9px;
    width: 180px;
    height: 180px;
    padding: 0;
    border: 1px solid rgba(255, 255, 255, 0.25);
    border-radius: 8px;
    overflow: hidden;
    cursor: pointer;
    background: #000;
    opacity: 0.92;
    pointer-events: auto;
  }
  .minimap.lowered {
    top: 58px;
  }
  .minimap:hover {
    border-color: rgba(255, 255, 255, 0.5);
  }
  canvas {
    display: block;
  }
</style>

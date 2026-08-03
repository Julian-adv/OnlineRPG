<script lang="ts">
  import { gameStore } from '../stores/gameStore'
  import { worldMapVisible } from '../stores/debugStore'
  import { minimapVersion } from '../stores/editorStore'
  import { currentDungeonDepth } from '../stores/dungeonStore'
  import { playerVisualFloorLevel } from '../stores/housingStore'
  import { regionMinimapServerUrl } from '../terrain/regionMinimapGenerator'
  import { REGION_CELLS, TILE_DIM } from '../terrain/terrain-constants'

  /** Canvas size in CSS pixels. */
  const SIZE = 180
  /** World meters shown across the widget — fixed zoom, sized so the region
   *  bakes (1 px/m) only ever downscale. */
  const VIEW_WORLD = 384
  /** Same map rotation as WorldMapDialog: screen-up matches walking up. */
  const ROTATE_ANGLE = -Math.PI / 4
  /** Player movement below this step doesn't trigger a redraw. */
  const REDRAW_STEP_M = 2
  /** Heading changes below this step (~10°) don't trigger a redraw. */
  const REDRAW_STEP_RAD = Math.PI / 18
  /** ~4 regions cover the rotated view; keep a little slack for movement. */
  const IMAGE_CACHE_LIMIT = 12

  let canvasEl = $state<HTMLCanvasElement>()

  /** The bakes are surface terrain: underground or upstairs there is nothing
   *  truthful to draw, so the widget hides rather than showing the surface. */
  let onSurface = $derived(
    $currentDungeonDepth === 0 && $playerVisualFloorLevel === 0
  )

  // Intentionally non-reactive: image loads must not re-run the render effect.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const imageCache = new Map<string, HTMLImageElement | null>()
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const pendingLoads = new Map<string, Promise<HTMLImageElement | null>>()

  function trimImageCache() {
    for (const key of imageCache.keys()) {
      if (imageCache.size <= IMAGE_CACHE_LIMIT) break
      imageCache.delete(key)
    }
  }

  function loadRegionImage(
    rx: number,
    rz: number
  ): Promise<HTMLImageElement | null> {
    const key = `${rx},${rz}`
    if (imageCache.has(key)) return Promise.resolve(imageCache.get(key)!)
    if (pendingLoads.has(key)) return pendingLoads.get(key)!

    const promise = new Promise<HTMLImageElement | null>((resolve) => {
      const img = new Image()
      img.onload = () => {
        imageCache.set(key, img)
        trimImageCache()
        pendingLoads.delete(key)
        resolve(img)
      }
      img.onerror = () => {
        imageCache.set(key, null)
        pendingLoads.delete(key)
        resolve(null)
      }
      img.src = regionMinimapServerUrl(rx, rz)
    })
    pendingLoads.set(key, promise)
    return promise
  }

  $effect(() => {
    const _ver = $minimapVersion // flush when the editor regenerates bakes
    imageCache.clear()
    pendingLoads.clear()
  })

  // Quantized player state: redraw on ~2 m moves or ~10° turns, not per frame.
  let playerX = $derived($gameStore.currentPlayer?.position.x ?? 0)
  let playerZ = $derived($gameStore.currentPlayer?.position.z ?? 0)
  let qx = $derived(Math.round(playerX / REDRAW_STEP_M))
  let qz = $derived(Math.round(playerZ / REDRAW_STEP_M))
  let qr = $derived(
    Math.round(($gameStore.currentPlayer?.rotation ?? 0) / REDRAW_STEP_RAD)
  )

  let renderGeneration = 0

  $effect(() => {
    if (!canvasEl) return

    // Reactive triggers: quantized pose, party snapshot, regenerated bakes.
    const px = qx * REDRAW_STEP_M
    const pz = qz * REDRAW_STEP_M
    const heading = qr * REDRAW_STEP_RAD
    const _ver = $minimapVersion
    const gen = ++renderGeneration

    const dpr = window.devicePixelRatio || 1
    canvasEl.width = SIZE * dpr
    canvasEl.height = SIZE * dpr
    const ctx = canvasEl.getContext('2d')!
    ctx.scale(dpr, dpr)

    const scale = SIZE / VIEW_WORLD
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
      ctx.rotate(ROTATE_ANGLE)
      ctx.translate(-SIZE / 2, -SIZE / 2)
      draw()
      ctx.restore()
    }

    const promises: Promise<void>[] = []
    for (let rz = minRz; rz <= maxRz; rz++) {
      for (let rx = minRx; rx <= maxRx; rx++) {
        const regionWorldX = rx * REGION_CELLS - TILE_DIM / 2
        const regionWorldZ = rz * REGION_CELLS - TILE_DIM / 2
        const drawX = Math.floor((regionWorldX - viewLeft) * scale)
        const drawY = Math.floor((regionWorldZ - viewTop) * scale)
        const drawSize = Math.ceil(REGION_CELLS * scale)
        promises.push(
          loadRegionImage(rx, rz).then((img) => {
            if (gen !== renderGeneration || !img) return
            rotated(() => ctx.drawImage(img, drawX, drawY, drawSize, drawSize))
          })
        )
      }
    }

    Promise.all(promises).then(() => {
      if (gen !== renderGeneration) return

      // Self: centered heading arrow. rotation = atan2(dx, dz), so the facing
      // vector in (x, z) is (sin r, cos r); the rotated transform handles the
      // map's -45° for us.
      rotated(() => {
        const angle = Math.atan2(Math.cos(heading), Math.sin(heading))
        ctx.translate(SIZE / 2, SIZE / 2)
        ctx.rotate(angle)
        ctx.beginPath()
        ctx.moveTo(7, 0)
        ctx.lineTo(-5, 5)
        ctx.lineTo(-2.5, 0)
        ctx.lineTo(-5, -5)
        ctx.closePath()
        ctx.fillStyle = '#ff3333'
        ctx.fill()
        ctx.lineWidth = 1.5
        ctx.strokeStyle = '#ffffff'
        ctx.stroke()
      })
    })
  })
</script>

{#if onSurface}
  <button
    class="minimap"
    title="Open world map (M)"
    aria-label="Open world map"
    onclick={() => worldMapVisible.set(true)}
  >
    <canvas bind:this={canvasEl} style="width: {SIZE}px; height: {SIZE}px;"
    ></canvas>
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
  .minimap:hover {
    border-color: rgba(255, 255, 255, 0.5);
  }
  canvas {
    display: block;
  }
</style>

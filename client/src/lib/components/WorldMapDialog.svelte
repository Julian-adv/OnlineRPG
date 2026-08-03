<script module lang="ts">
  import mapLabelsJson from '../../../../data/map_labels.json'
  import { RegionImageCache } from '../terrain/regionImageCache'

  const REGION_SIZE = 16
  const TILE_DIM = 64
  const REGION_PX = REGION_SIZE * TILE_DIM // 1024

  const MIN_ZOOM = 1
  const DEFAULT_ZOOM = 8

  // Party member marker poll cadence; above the server's 2s clamp.
  const PARTY_POLL_MS = 3000

  // A poll answer can outlive the dialog that requested it (nothing polls
  // while the map is closed); older data than this never renders.
  const PARTY_POSITIONS_MAX_AGE_MS = 10000

  // --- Shared place-name labels (generated from data-src/map_labels.csv,
  // plus the player's discovered dungeon entrances) ---
  type LabelKind =
    | 'continent'
    | 'capital'
    | 'city'
    | 'town'
    | 'sea'
    | 'island'
    | 'dungeon'
  interface MapLabel {
    /** Stable each-key, unique across kinds (names may repeat between them). */
    key: string
    name: string
    kind: LabelKind
    x: number // world meters
    z: number
  }
  const MAP_LABELS: MapLabel[] = Object.values(
    mapLabelsJson as unknown as Record<string, Omit<MapLabel, 'key'>>
  ).map((label) => ({ ...label, key: `${label.kind}:${label.name}` }))

  // Per-kind zoom visibility: shown when min <= zoomSpan <= max (zoomSpan = regions
  // across; larger = zoomed out). Continents/seas appear when zoomed out, settlements
  // when zoomed in.
  // Settlements (capital/city/town) and dungeons share the same max so they
  // all appear together at the zoom where the capital is visible.
  const LABEL_ZOOM: Record<LabelKind, { min: number; max: number }> = {
    continent: { min: 8, max: Infinity },
    sea: { min: 4, max: Infinity },
    capital: { min: 1, max: 24 },
    city: { min: 1, max: 24 },
    town: { min: 1, max: 24 },
    island: { min: 1, max: 16 },
    dungeon: { min: 1, max: 24 },
  }

  // Matches the canvas's -45deg map rotation, applied to label screen positions.
  const ROTATE_ANGLE = -Math.PI / 4
  const COS_R = Math.cos(ROTATE_ANGLE)
  const SIN_R = Math.sin(ROTATE_ANGLE)

  // Module-level: images persist across dialog open/close.
  const regionImages = new RegionImageCache()

  // --- Persisted view state (survives dialog close/reopen) ---
  let savedCamX: number | null = null
  let savedCamZ: number | null = null
  let savedZoom: number | null = null
</script>

<script lang="ts">
  import { gameStore, isAdminUser } from '../stores/gameStore'
  import {
    partyRoster,
    partyPositions,
    resetPartyPositions,
  } from '../stores/partyStore'
  import { worldMapVisible, teleportLoading } from '../stores/debugStore'
  import { discoveredDungeonIds } from '../stores/dungeonStore'
  import { houseMapFootprints } from '../stores/housingMapStore'
  import { DUNGEON_ENTRANCES } from '../data/dungeonDefs'
  import { minimapVersion } from '../stores/editorStore'
  import { networkManager } from '../network/socket'
  import {
    graphicsQuality,
    getEffectivePreset,
  } from '../stores/graphicsSettings'
  import { wrapWorldX, unwrapWorldXNear } from '../terrain/world-wrap'
  import { mountOverlay } from '../stores/overlayStack'
  import { drawHouseMapFootprints } from '../utils/map-structures'

  const graphicsPreset = $derived(getEffectivePreset($graphicsQuality))
  const mobileMapBudget = $derived(graphicsPreset.renderBudget === 'mobile')
  const defaultZoomSpan = $derived(graphicsPreset.worldMapDefaultZoomSpan)
  const maxZoomSpan = $derived(graphicsPreset.worldMapMaxZoomSpan)
  const imageCacheLimit = $derived(graphicsPreset.worldMapImageCacheLimit)

  $effect(() => {
    regionImages.limit = imageCacheLimit
  })

  // --- Component state ---
  let containerEl = $state<HTMLDivElement>()
  let canvasEl = $state<HTMLCanvasElement>()
  let containerW = $state(0)
  let containerH = $state(0)

  let playerX = $derived(wrapWorldX($gameStore.currentPlayer?.position.x ?? 0))
  let playerZ = $derived($gameStore.currentPlayer?.position.z ?? 0)
  let playerHeading = $derived($gameStore.currentPlayer?.rotation ?? 0)

  // --- Camera state (world coordinates of view center) ---
  let camX = $state(0)
  let camZ = $state(0)

  // --- Zoom state (in regions/km) ---
  let zoomSpan = $state(DEFAULT_ZOOM)
  let teleportMode = $state(false)

  // Restore saved view state or center on player when dialog opens
  $effect(() => {
    if ($worldMapVisible) {
      if (savedCamX !== null && savedCamZ !== null) {
        camX = savedCamX
        camZ = savedCamZ
      } else {
        camX = playerX
        camZ = playerZ
      }
      if (savedZoom !== null) {
        zoomSpan = Math.min(savedZoom, maxZoomSpan)
      } else {
        zoomSpan = defaultZoomSpan
      }
    }
  })

  // Party existence only: roster churn must not reset the poll cadence (the
  // immediate re-poll would just be eaten by the server's clamp).
  let inParty = $derived($partyRoster !== null)

  // Poll party positions only while the dialog lives (it mounts with
  // worldMapVisible) and a party exists — closed map means a silent channel.
  $effect(() => {
    if (!inParty) return
    networkManager.sendRequestPartyPositions()
    const timer = setInterval(
      () => networkManager.sendRequestPartyPositions(),
      PARTY_POLL_MS
    )
    return () => clearInterval(timer)
  })

  // The render-time age gate only runs when something recomputes; this makes
  // expiry certain on an untouched map (same pattern as PartyInviteToast).
  $effect(() => {
    const at = $partyPositions.at
    if (at === 0) return
    const timer = setTimeout(
      resetPartyPositions,
      Math.max(0, at + PARTY_POSITIONS_MAX_AGE_MS - Date.now())
    )
    return () => clearTimeout(timer)
  })

  // --- Drag state ---
  let isDragging = $state(false)
  let suppressNextClick = false
  // Squared pixel distance a pointer must travel before a drag suppresses the
  // click that would otherwise fire on pointerup.
  const DRAG_THRESHOLD_PX2 = 9
  let dragStartMouseX = 0
  let dragStartMouseZ = 0
  let dragStartCamX = 0
  let dragStartCamZ = 0

  // --- Canvas rendering ---
  let renderGeneration = 0

  $effect(() => {
    if (!canvasEl || containerW <= 0 || containerH <= 0) return

    const mmVer = $minimapVersion // re-render when minimaps change
    const span = zoomSpan
    const cx = camX
    const cz = camZ
    const houses = $houseMapFootprints
    const cw = containerW
    const ch = containerH
    const gen = ++renderGeneration

    const ctx = canvasEl.getContext('2d')!

    // Scale: how many canvas pixels per world unit
    // At current zoom, we show `span` regions across the shorter dimension
    const viewSize = span * REGION_PX // world units visible along shorter axis
    const canvasSize = Math.min(cw, ch)
    const scale = canvasSize / viewSize

    // World-space extents of the viewport
    const viewWorldW = cw / scale
    const viewWorldH = ch / scale

    // World-space top-left of viewport
    const viewLeft = cx - viewWorldW / 2
    const viewTop = cz - viewWorldH / 2

    // Clear to black
    ctx.clearRect(0, 0, cw, ch)
    ctx.fillStyle = '#000'
    ctx.fillRect(0, 0, cw, ch)

    // 45-degree rotation: expand visible region to cover rotated corners
    const expand = Math.SQRT2 // rotated square needs ~1.41x coverage

    const expandedViewWorldW = viewWorldW * expand
    const expandedViewWorldH = viewWorldH * expand
    const expandedViewLeft = cx - expandedViewWorldW / 2
    const expandedViewTop = cz - expandedViewWorldH / 2

    const expRegionMinRx = Math.floor(
      (expandedViewLeft + TILE_DIM / 2) / REGION_PX
    )
    const expRegionMaxRx = Math.floor(
      (expandedViewLeft + expandedViewWorldW + TILE_DIM / 2) / REGION_PX
    )
    const expRegionMinRz = Math.floor(
      (expandedViewTop + TILE_DIM / 2) / REGION_PX
    )
    const expRegionMaxRz = Math.floor(
      (expandedViewTop + expandedViewWorldH + TILE_DIM / 2) / REGION_PX
    )

    const promises: Promise<void>[] = []
    for (let rz = expRegionMinRz; rz <= expRegionMaxRz; rz++) {
      for (let rx = expRegionMinRx; rx <= expRegionMaxRx; rx++) {
        // Region world origin
        const regionWorldX = rx * REGION_PX - TILE_DIM / 2
        const regionWorldZ = rz * REGION_PX - TILE_DIM / 2

        // Canvas position (before rotation, relative to view center)
        const drawX = Math.floor((regionWorldX - viewLeft) * scale)
        const drawY = Math.floor((regionWorldZ - viewTop) * scale)
        const drawSize = Math.ceil(REGION_PX * scale)

        promises.push(
          regionImages.load(rx, rz, mmVer).then((img) => {
            if (gen !== renderGeneration) return
            if (img) {
              ctx.save()
              ctx.translate(cw / 2, ch / 2)
              ctx.rotate(ROTATE_ANGLE)
              ctx.translate(-cw / 2, -ch / 2)
              ctx.drawImage(img, drawX, drawY, drawSize, drawSize)
              ctx.restore()
            }
          })
        )
      }
    }

    Promise.all(promises).then(() => {
      if (gen !== renderGeneration) return

      ctx.save()
      ctx.translate(cw / 2, ch / 2)
      ctx.rotate(ROTATE_ANGLE)
      ctx.translate(-cw / 2, -ch / 2)
      drawHouseMapFootprints(ctx, houses, {
        centerX: cx,
        viewLeft,
        viewTop,
        scale,
      })
      ctx.restore()
    })
  })

  // --- Place-name label overlay (HTML layer, not burned into the canvas) ---
  interface PlacedLabel {
    key: string
    name: string
    kind: LabelKind
    left: number
    top: number
  }

  // World → overlay coords: unwrap x toward the camera (a point just across
  // the world seam renders near the edge instead of a full wrap away), scale
  // around the view center, then the same -45° rotation the canvas applies
  // (ctx.rotate(ROTATE_ANGLE)).
  function worldToScreen(x: number, z: number, cw: number, ch: number) {
    x = unwrapWorldXNear(camX, x)
    const scale = Math.min(cw, ch) / (zoomSpan * REGION_PX)
    const lx = (x - (camX - cw / scale / 2)) * scale
    const ly = (z - (camZ - ch / scale / 2)) * scale
    const ox = lx - cw / 2
    const oy = ly - ch / 2
    return {
      left: ox * COS_R - oy * SIN_R + cw / 2,
      top: ox * SIN_R + oy * COS_R + ch / 2,
    }
  }

  function onScreen(
    p: { left: number; top: number },
    cw: number,
    ch: number,
    margin: number
  ) {
    return (
      p.left >= -margin &&
      p.left <= cw + margin &&
      p.top >= -margin &&
      p.top <= ch + margin
    )
  }

  // Static place names plus the player's discovered dungeon entrances, so
  // dungeons ride the same zoom/cull/label pipeline as every other kind.
  let mapLabels = $derived.by<MapLabel[]>(() => {
    const known = $discoveredDungeonIds
    if (known.size === 0) return MAP_LABELS
    const dungeons = DUNGEON_ENTRANCES.filter((e) => known.has(e.id)).map(
      (e) => ({
        key: `dungeon:${e.id}`,
        name: e.name,
        kind: 'dungeon' as const,
        x: e.x,
        z: e.z,
      })
    )
    return [...MAP_LABELS, ...dungeons]
  })

  let visibleLabels = $derived.by<PlacedLabel[]>(() => {
    const cw = containerW
    const ch = containerH
    if (cw <= 0 || ch <= 0) return []

    const margin = 80 // keep labels whose anchor is just off-edge
    const out: PlacedLabel[] = []
    for (const label of mapLabels) {
      const tier = LABEL_ZOOM[label.kind]
      if (zoomSpan < tier.min || zoomSpan > tier.max) continue
      const p = worldToScreen(label.x, label.z, cw, ch)
      if (!onScreen(p, cw, ch, margin)) continue
      out.push({
        key: label.key,
        name: label.name,
        kind: label.kind,
        left: p.left,
        top: p.top,
      })
    }
    return out
  })

  let selfMarker = $derived.by<{
    left: number
    top: number
    angle: number
  } | null>(() => {
    const cw = containerW
    const ch = containerH
    if (cw <= 0 || ch <= 0) return null
    const p = worldToScreen(playerX, playerZ, cw, ch)
    if (!onScreen(p, cw, ch, 20)) return null
    return {
      ...p,
      angle:
        Math.atan2(Math.cos(playerHeading), Math.sin(playerHeading)) +
        ROTATE_ANGLE,
    }
  })

  // --- Party member markers (HTML layer, same transform as the labels) ---
  interface PartyMarker {
    id: number
    name: string
    left: number
    top: number
    floor: number
  }

  let partyMarkers = $derived.by<PartyMarker[]>(() => {
    const roster = $partyRoster
    const positions = $partyPositions
    const cw = containerW
    const ch = containerH
    if (!roster || cw <= 0 || ch <= 0) return []
    if (Date.now() - positions.at > PARTY_POSITIONS_MAX_AGE_MS) return []

    // Join against the roster: a member who left since the last poll (or an
    // id the roster never knew) must not draw a ghost.
    const names = new Map(roster.members.map((m) => [m.id, m.name]))
    const out: PartyMarker[] = []
    for (const pos of positions.members) {
      const name = names.get(pos.id)
      if (!name) continue
      const p = worldToScreen(pos.x, pos.z, cw, ch)
      if (!onScreen(p, cw, ch, 40)) continue
      out.push({
        id: pos.id,
        name,
        left: p.left,
        top: p.top,
        floor: pos.floor_level,
      })
    }
    return out
  })

  // --- Zoom controls ---
  function zoomIn() {
    zoomSpan = Math.max(MIN_ZOOM, zoomSpan - 1)
  }

  function zoomOut() {
    zoomSpan = Math.min(maxZoomSpan, zoomSpan + 1)
  }

  function zoomReset() {
    zoomSpan = defaultZoomSpan
    savedZoom = null
  }

  function resetCamera() {
    camX = playerX
    camZ = playerZ
    savedCamX = null
    savedCamZ = null
  }

  function handleWheel(event: WheelEvent) {
    event.preventDefault()
    if (event.deltaY > 0) {
      zoomOut()
    } else {
      zoomIn()
    }
  }

  $effect(() => {
    if (!containerEl) return
    containerEl.addEventListener('wheel', handleWheel, { passive: false })
    return () => containerEl!.removeEventListener('wheel', handleWheel)
  })

  // --- Drag to pan ---
  function handlePointerDown(event: PointerEvent) {
    if (event.ctrlKey && $isAdminUser) return // let Ctrl+click through for teleport
    if (event.pointerType === 'mouse' && event.button !== 0) return
    event.preventDefault()
    isDragging = true
    suppressNextClick = false
    dragStartMouseX = event.clientX
    dragStartMouseZ = event.clientY
    dragStartCamX = camX
    dragStartCamZ = camZ
  }

  function handlePointerMove(event: PointerEvent) {
    if (!isDragging) return
    event.preventDefault()
    const viewSize = zoomSpan * REGION_PX
    const canvasSize = Math.min(containerW, containerH)
    const scale = canvasSize / viewSize

    // Rotate mouse delta by +45 degrees to undo the canvas rotation
    const rawDx = event.clientX - dragStartMouseX
    const rawDz = event.clientY - dragStartMouseZ
    if (
      !suppressNextClick &&
      rawDx * rawDx + rawDz * rawDz > DRAG_THRESHOLD_PX2
    ) {
      suppressNextClick = true
    }
    const dx = rawDx / scale
    const dz = rawDz / scale
    const angle = Math.PI / 4
    const cosA = Math.cos(angle)
    const sinA = Math.sin(angle)
    camX = dragStartCamX - (dx * cosA - dz * sinA)
    camZ = dragStartCamZ - (dx * sinA + dz * cosA)
  }

  function handlePointerUp() {
    isDragging = false
  }

  $effect(() => {
    if (!isDragging) return
    window.addEventListener('pointermove', handlePointerMove, {
      passive: false,
    })
    window.addEventListener('pointerup', handlePointerUp)
    window.addEventListener('pointercancel', handlePointerUp)
    return () => {
      window.removeEventListener('pointermove', handlePointerMove)
      window.removeEventListener('pointerup', handlePointerUp)
      window.removeEventListener('pointercancel', handlePointerUp)
    }
  })

  // Save view state on component destroy (covers all close paths)
  $effect(() => {
    return () => {
      savedCamX = camX
      savedCamZ = camZ
      savedZoom = zoomSpan
    }
  })

  // --- Actions ---
  function close() {
    if (mobileMapBudget) {
      renderGeneration++
      regionImages.flush()
    }
    teleportMode = false
    worldMapVisible.set(false)
  }

  // Registering `close` keeps the mobile cache teardown on Escape.
  $effect(() => mountOverlay('worldMap', close))

  function handleBackdropClick(event: MouseEvent) {
    if (event.target === event.currentTarget) {
      close()
    }
  }

  function handleMapClick(event: MouseEvent) {
    if (suppressNextClick) {
      suppressNextClick = false
      event.preventDefault()
      event.stopPropagation()
      return
    }
    const teleportRequested = (event.ctrlKey || teleportMode) && $isAdminUser
    if (!teleportRequested) return
    event.preventDefault()
    event.stopPropagation()
    teleportAt(event.clientX, event.clientY)
  }

  // macOS turns Ctrl+click into a contextmenu event (no click fires at all),
  // so the teleport shortcut must be caught here too.
  function handleMapContextMenu(event: MouseEvent) {
    if (!event.ctrlKey || !$isAdminUser) return
    event.preventDefault()
    event.stopPropagation()
    teleportAt(event.clientX, event.clientY)
  }

  function teleportAt(clientX: number, clientY: number) {
    if (!$isAdminUser) return
    if (!containerEl || containerW <= 0 || containerH <= 0) return

    const rect = containerEl.getBoundingClientRect()
    const pixelX = clientX - rect.left
    const pixelY = clientY - rect.top

    const viewSize = zoomSpan * REGION_PX
    const canvasSize = Math.min(containerW, containerH)
    const scale = canvasSize / viewSize

    // Screen offset from center, then rotate by +45 degrees to undo canvas rotation
    const sx = (pixelX - containerW / 2) / scale
    const sz = (pixelY - containerH / 2) / scale
    const angle = Math.PI / 4
    const cosA = Math.cos(angle)
    const sinA = Math.sin(angle)
    const worldX = wrapWorldX(camX + (sx * cosA - sz * sinA))
    const worldZ = camZ + (sx * sinA + sz * cosA)

    const position = { x: worldX, y: 0, z: worldZ }

    gameStore.update((state) => {
      if (!state.currentPlayer) return state
      state.currentPlayer.position.set(worldX, 0, worldZ)
      return state
    })

    networkManager.sendDebugTeleport(position)
    teleportLoading.set(true)
    close()
  }

  // --- Resize observer ---
  $effect(() => {
    if (!containerEl) return
    const ro = new ResizeObserver((entries) => {
      const entry = entries[0]
      if (entry) {
        containerW = entry.contentRect.width
        containerH = entry.contentRect.height
      }
    })
    ro.observe(containerEl)
    return () => ro.disconnect()
  })
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onclick={handleBackdropClick}>
  <div
    class="dialog"
    class:mobile-map-budget={mobileMapBudget}
    role="dialog"
    aria-modal="true"
  >
    <div class="header">
      <h2>World Map</h2>
      <div class="controls">
        <button class="ctrl-btn" onclick={zoomIn} title="Zoom In">+</button>
        <button class="ctrl-btn" onclick={zoomOut} title="Zoom Out"
          >&minus;</button
        >
        <button class="ctrl-btn" onclick={zoomReset} title="Reset Zoom"
          >Reset</button
        >
        <button class="ctrl-btn" onclick={resetCamera} title="Center on Player"
          >&#8982;</button
        >
        {#if $isAdminUser}
          <button
            class="ctrl-btn"
            class:active={teleportMode}
            onclick={() => (teleportMode = !teleportMode)}
            title="Teleport Mode">TP</button
          >
        {/if}
      </div>
      <button class="close-btn" onclick={close}>&times;</button>
    </div>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="map-container"
      class:dragging={isDragging}
      bind:this={containerEl}
      onpointerdown={handlePointerDown}
      onclick={handleMapClick}
      oncontextmenu={handleMapContextMenu}
    >
      <canvas
        bind:this={canvasEl}
        width={containerW}
        height={containerH}
        class="map-canvas"
      ></canvas>
      <div class="label-layer">
        {#each visibleLabels as label (label.key)}
          <div
            class="map-label {label.kind}"
            class:area={label.kind === 'continent' ||
              label.kind === 'sea' ||
              label.kind === 'island'}
            style="left: {label.left}px; top: {label.top}px;"
          >
            {#if label.kind !== 'continent' && label.kind !== 'sea' && label.kind !== 'island'}
              <span class="marker"></span>
            {/if}
            <span class="text">{label.name}</span>
          </div>
        {/each}
        {#each partyMarkers as marker (marker.id)}
          <div
            class="party-marker"
            style="left: {marker.left}px; top: {marker.top}px;"
          >
            <span class="dot"></span>
            <span class="text"
              >{marker.name}{marker.floor < 0 ? ` B${-marker.floor}` : ''}</span
            >
          </div>
        {/each}
        {#if selfMarker}
          <svg
            class="self-marker"
            style="left: {selfMarker.left}px; top: {selfMarker.top}px; transform: translate(-50%, -50%) rotate({selfMarker.angle}rad);"
            viewBox="-7 -7 16 14"
            aria-hidden="true"
          >
            <path d="M 8 0 L -6 6 L -3 0 L -6 -6 Z"></path>
          </svg>
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.6);
    z-index: 30;
  }

  .dialog {
    width: min(80vw, 800px);
    height: min(80vh, 800px);
    display: flex;
    flex-direction: column;
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    background: rgba(16, 16, 16, 0.95);
    color: #f4f4f4;
    overflow: hidden;
  }

  .dialog.mobile-map-budget {
    width: calc(
      100vw - 16px - env(safe-area-inset-left) - env(safe-area-inset-right)
    );
    height: min(
      calc(
        100dvh - 96px - env(safe-area-inset-top) - env(safe-area-inset-bottom)
      ),
      calc(
        100vw - 16px - env(safe-area-inset-left) - env(safe-area-inset-right)
      )
    );
    max-width: 440px;
    max-height: 440px;
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
  }

  .header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }

  .controls {
    display: flex;
    gap: 4px;
  }

  .ctrl-btn {
    background: rgba(255, 255, 255, 0.1);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 4px;
    color: #ccc;
    font-size: 14px;
    cursor: pointer;
    padding: 2px 8px;
    line-height: 1.4;
  }

  .ctrl-btn:hover {
    background: rgba(255, 255, 255, 0.2);
    color: #fff;
  }

  .ctrl-btn.active {
    border-color: rgba(240, 192, 64, 0.65);
    background: rgba(240, 192, 64, 0.2);
    color: #ffe08a;
  }

  .close-btn {
    background: none;
    border: none;
    color: #aaa;
    font-size: 22px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #fff;
  }

  .map-container {
    flex: 1;
    position: relative;
    min-height: 0;
    overflow: hidden;
    cursor: grab;
    touch-action: none;
    user-select: none;
  }

  .map-container.dragging {
    cursor: grabbing;
  }

  .map-canvas {
    position: absolute;
    inset: 0;
    display: block;
  }

  /* Place-name labels: HTML overlay above the canvas, clicks pass through. */
  .label-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    /* dark halo for readability over varied terrain */
    --label-halo:
      0 0 2px #000, 1px 1px 1px #000, -1px 1px 1px #000, 1px -1px 1px #000,
      -1px -1px 1px #000;
  }

  .map-label {
    position: absolute;
    user-select: none;
  }

  .map-label .marker {
    position: absolute;
    left: 0;
    top: 0;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    box-sizing: border-box;
  }

  .map-label .text {
    position: absolute;
    left: 0;
    top: 0;
    white-space: nowrap;
    font-family: Georgia, 'Times New Roman', serif;
    text-shadow: var(--label-halo);
  }

  /* point kinds (capital/city/town): marker centered on anchor, text to the right */
  .map-label:not(.area) .text {
    transform: translate(11px, -50%);
  }

  /* area kinds (continent/sea): centered label, no marker */
  .map-label.area .text {
    transform: translate(-50%, -50%);
    text-align: center;
  }

  .map-label.continent .text {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: 3px;
    color: #f6f0e2;
  }

  .map-label.sea .text {
    font-size: 16px;
    font-style: italic;
    letter-spacing: 1px;
    color: #7ec8f0;
  }

  .map-label.island .text {
    font-size: 13px;
    font-weight: 600;
    color: #d6e6cf;
  }

  .map-label.capital .text {
    font-size: 16px;
    font-weight: 700;
    color: #fffcf4;
  }

  .map-label.city .text {
    font-size: 14px;
    font-weight: 700;
    color: #fffcf4;
  }

  .map-label.town .text {
    font-size: 14px;
    font-weight: 700;
    color: #fff6dc;
  }

  .map-label.capital .marker {
    width: 13px;
    height: 13px;
    background: #fad746;
    border: 2px solid #19120a;
    box-shadow: 0 0 0 3px rgba(25, 18, 10, 0.55);
  }

  .map-label.city .marker {
    width: 10px;
    height: 10px;
    background: #f5d250;
    border: 2px solid #19120a;
  }

  .map-label.town .marker {
    width: 11px;
    height: 11px;
    background: #f5d250;
    border: 2px solid #287832;
  }

  .map-label.dungeon .text {
    font-size: 13px;
    font-weight: 700;
    color: #e8b8a8;
  }

  /* Discovered dungeon entrance: a dark diamond, styled like the settlement
     dots but unmistakably not a place to live. */
  .map-label.dungeon .marker {
    width: 9px;
    height: 9px;
    border-radius: 0;
    transform: translate(-50%, -50%) rotate(45deg);
    background: #35333d;
    border: 2px solid #b0503c;
    box-shadow: 0 0 4px rgba(176, 80, 60, 0.7);
  }

  .self-marker {
    position: absolute;
    z-index: 3;
    width: 16px;
    height: 14px;
    overflow: visible;
    filter: drop-shadow(0 0 3px rgba(255, 50, 50, 0.8));
  }

  .self-marker path {
    fill: #ff3333;
    stroke: #fff;
    stroke-width: 1.5;
    stroke-linejoin: round;
  }

  .party-marker {
    position: absolute;
    user-select: none;
  }

  .party-marker .dot {
    position: absolute;
    left: 0;
    top: 0;
    transform: translate(-50%, -50%);
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: #3fa7ff;
    border: 2px solid #fff;
    box-sizing: border-box;
    box-shadow: 0 0 6px rgba(63, 167, 255, 0.8);
  }

  .party-marker .text {
    position: absolute;
    left: 0;
    top: 0;
    transform: translate(10px, -50%);
    white-space: nowrap;
    font-size: 12px;
    font-weight: 700;
    color: #bfe0ff;
    text-shadow: var(--label-halo);
  }
</style>

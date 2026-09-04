<script module lang="ts">
  import {
    MAP_LABELS as MAP_LABEL_DEFS,
    type MapLabelDef,
    type MapLabelKind,
  } from '../data/mapLabels'
  import { RegionImageCache } from '../terrain/regionImageCache'
  import {
    getMapFrameCornerReservedBounds,
    isFixedMapLabel,
    layoutMapLabels,
    type ScreenRect,
  } from '../utils/worldMapLabelLayout'
  import { pickMinimapSourceSize } from '../terrain/regionMinimapGenerator'
  import {
    persistWorldMapView,
    resolveWorldMapView,
  } from '../utils/worldMapViewState'

  const REGION_SIZE = 16
  const TILE_DIM = 64
  const REGION_PX = REGION_SIZE * TILE_DIM // 1024
  const ATLAS_PADDING_PX = 2
  /** Floor on the image-cache size for LODs small enough to keep many of. */
  const COARSE_CACHE_LIMIT: Record<number, number> = { 128: 1024, 256: 512 }
  // Average deep-sea color of the baked fantasy tiles, shown past the world edge
  const OUT_OF_WORLD_OCEAN = '#01294e'

  const MIN_ZOOM = 1
  const DEFAULT_ZOOM = 16
  const MOBILE_AREA_ZOOM_REFERENCE = 8

  // --- Place-name labels, plus the player's discovered dungeon entrances ---
  type LabelKind = MapLabelKind
  interface MapLabel extends MapLabelDef {
    /** Stable each-key, unique across kinds (names may repeat between them). */
    key: string
  }
  const MAP_LABELS: MapLabel[] = MAP_LABEL_DEFS.map((label) => ({
    ...label,
    key: `${label.kind}:${label.id}`,
  }))

  // Matches the canvas's -45deg map rotation, applied to label screen positions.
  const COS_R = Math.cos(MAP_ROTATE_ANGLE)
  const SIN_R = Math.sin(MAP_ROTATE_ANGLE)

  /** Undo the canvas rotation: a screen-space offset in world units becomes a
   *  world-space offset. */
  function screenDeltaToWorld(dx: number, dz: number) {
    return { x: dx * COS_R + dz * SIN_R, z: -dx * SIN_R + dz * COS_R }
  }

  // Module-level: images persist across dialog open/close.
  const regionImages = new RegionImageCache()

  // --- Persisted view state (survives dialog close/reopen) ---
  let savedCamX: number | null = null
  let savedCamZ: number | null = null
  let savedZoom: number | null = null
  let followPlayerOnOpen = true
  let useDefaultZoomOnOpen = true
</script>

<script lang="ts">
  import { assetUrl } from '../utils/assetUrl'
  import { gameStore, isAdminUser } from '../stores/gameStore'
  import { partyRoster, partyPositions } from '../stores/partyStore'
  import { worldMapVisible, gridVisible } from '../stores/debugStore'
  import { discoveredDungeonIds } from '../stores/dungeonStore'
  import { houseMapFootprints } from '../stores/housingMapStore'
  import { DUNGEON_ENTRANCES } from '../data/dungeonDefs'
  import { minimapVersion } from '../stores/editorStore'
  import { networkManager } from '../network/socket'
  import {
    graphicsQuality,
    getEffectivePreset,
  } from '../stores/graphicsSettings'
  import {
    wrapWorldX,
    unwrapWorldXNear,
    WORLD_MIN_REGION_Z,
    WORLD_MAX_REGION_Z,
  } from '../terrain/world-wrap'
  import { mountOverlay } from '../stores/overlayStack'
  import {
    MAP_ROTATE_ANGLE,
    drawHouseMapFootprints,
    drawLandPlotGrid,
    headingToMapAngle,
  } from '../utils/map-structures'
  import { teleportLocalPlayer } from '../utils/teleport'
  import SelfMarker from './SelfMarker.svelte'

  const graphicsPreset = $derived(getEffectivePreset($graphicsQuality))
  const mobileMapBudget = $derived(graphicsPreset.renderBudget === 'mobile')
  const defaultZoomSpan = $derived(graphicsPreset.worldMapDefaultZoomSpan)
  const maxZoomSpan = $derived(graphicsPreset.worldMapMaxZoomSpan)
  const imageCacheLimit = $derived(graphicsPreset.worldMapImageCacheLimit)

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
  let initializedForOpen = $state(false)

  // Restore saved view state or center on player when dialog opens
  $effect(() => {
    if (!$worldMapVisible) {
      initializedForOpen = false
      return
    }
    if (initializedForOpen) return
    initializedForOpen = true

    const restored = resolveWorldMapView(
      { camX: savedCamX, camZ: savedCamZ, zoomSpan: savedZoom },
      { followPlayerOnOpen, useDefaultZoomOnOpen },
      { camX: playerX, camZ: playerZ, zoomSpan: defaultZoomSpan },
      maxZoomSpan
    )
    camX = restored.camX
    camZ = restored.camZ
    zoomSpan = restored.zoomSpan
  })

  // Party existence only: roster churn must not re-request (a membership
  // change already triggers a server push).
  let inParty = $derived($partyRoster !== null)

  // One snapshot when the dialog opens with a party, or a party forms while
  // it is open; steady-state updates are pushed by the server.
  $effect(() => {
    if (!inParty) return
    networkManager.sendRequestPartyPositions()
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
  interface RenderedView {
    camX: number
    camZ: number
    zoomSpan: number
    width: number
    height: number
  }

  let renderGeneration = 0
  let renderAtlas: HTMLCanvasElement | null = null
  let renderedView = $state<RenderedView | null>(null)

  $effect(() => {
    if (!canvasEl || containerW <= 0 || containerH <= 0) return

    const mmVer = $minimapVersion // re-render when minimaps change
    const span = zoomSpan
    const cx = camX
    const cz = camZ
    const houses = $houseMapFootprints
    const landGrid = $gridVisible
    const cw = containerW
    const ch = containerH
    const dpr = Math.min(
      window.devicePixelRatio || 1,
      graphicsPreset.pixelRatioCap
    )
    const gen = ++renderGeneration

    const backingW = Math.max(1, Math.round(cw * dpr))
    const backingH = Math.max(1, Math.round(ch * dpr))
    if (canvasEl.width !== backingW || canvasEl.height !== backingH) {
      canvasEl.width = backingW
      canvasEl.height = backingH
      renderedView = null
    }
    const ctx = canvasEl.getContext('2d')!
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.imageSmoothingEnabled = true
    ctx.imageSmoothingQuality = 'high'

    // Scale: how many canvas pixels per world unit
    // At current zoom, we show `span` regions across the shorter dimension
    const viewSize = span * REGION_PX // world units visible along shorter axis
    const canvasSize = Math.min(cw, ch)
    const scale = canvasSize / viewSize
    const projectedRegionPx = REGION_PX * scale * dpr
    const sourceSize = pickMinimapSourceSize(projectedRegionPx)
    // Coarse tiles are ~64x cheaper per image, so the preset's image budget
    // buys proportionally more of them.
    regionImages.limit = mobileMapBudget
      ? imageCacheLimit
      : Math.max(imageCacheLimit, COARSE_CACHE_LIMIT[sourceSize] ?? 0)

    // World-space extents of the viewport
    const viewWorldW = cw / scale
    const viewWorldH = ch / scale

    // World-space top-left of viewport
    const viewLeft = cx - viewWorldW / 2
    const viewTop = cz - viewWorldH / 2

    // Bounding square of the viewport after undoing its 45-degree rotation.
    const expandedViewWorldSize = (viewWorldW + viewWorldH) / Math.SQRT2
    const expandedViewLeft = cx - expandedViewWorldSize / 2
    const expandedViewTop = cz - expandedViewWorldSize / 2

    const expRegionMinRx = Math.floor(
      (expandedViewLeft + TILE_DIM / 2) / REGION_PX
    )
    const expRegionMaxRx = Math.floor(
      (expandedViewLeft + expandedViewWorldSize + TILE_DIM / 2) / REGION_PX
    )
    const expRegionMinRz = Math.floor(
      (expandedViewTop + TILE_DIM / 2) / REGION_PX
    )
    const expRegionMaxRz = Math.floor(
      (expandedViewTop + expandedViewWorldSize + TILE_DIM / 2) / REGION_PX
    )

    interface LoadedRegion {
      image: HTMLImageElement
      worldX: number
      worldZ: number
    }

    const promises: Promise<LoadedRegion | null>[] = []
    for (let rz = expRegionMinRz; rz <= expRegionMaxRz; rz++) {
      if (rz < WORLD_MIN_REGION_Z || rz > WORLD_MAX_REGION_Z) continue
      for (let rx = expRegionMinRx; rx <= expRegionMaxRx; rx++) {
        const regionWorldX = rx * REGION_PX - TILE_DIM / 2
        const regionWorldZ = rz * REGION_PX - TILE_DIM / 2

        promises.push(
          regionImages.load(rx, rz, mmVer, sourceSize).then((img) => {
            if (!img || gen !== renderGeneration) return null
            return {
              image: img,
              worldX: regionWorldX,
              worldZ: regionWorldZ,
            }
          })
        )
      }
    }

    Promise.all(promises).then((regions) => {
      if (gen !== renderGeneration) return

      const atlas = (renderAtlas ??= document.createElement('canvas'))
      const atlasWidth =
        Math.max(1, Math.ceil(expandedViewWorldSize * scale * dpr)) +
        ATLAS_PADDING_PX * 2
      const atlasHeight =
        Math.max(1, Math.ceil(expandedViewWorldSize * scale * dpr)) +
        ATLAS_PADDING_PX * 2
      if (atlas.width !== atlasWidth) atlas.width = atlasWidth
      if (atlas.height !== atlasHeight) atlas.height = atlasHeight
      const atlasCtx = atlas.getContext('2d')!
      atlasCtx.setTransform(1, 0, 0, 1, 0, 0)
      atlasCtx.clearRect(0, 0, atlas.width, atlas.height)
      atlasCtx.imageSmoothingEnabled = true
      atlasCtx.imageSmoothingQuality = 'high'

      for (const region of regions) {
        if (!region) continue
        const x0 =
          ATLAS_PADDING_PX +
          Math.round((region.worldX - expandedViewLeft) * scale * dpr)
        const y0 =
          ATLAS_PADDING_PX +
          Math.round((region.worldZ - expandedViewTop) * scale * dpr)
        const x1 =
          ATLAS_PADDING_PX +
          Math.round(
            (region.worldX + REGION_PX - expandedViewLeft) * scale * dpr
          )
        const y1 =
          ATLAS_PADDING_PX +
          Math.round(
            (region.worldZ + REGION_PX - expandedViewTop) * scale * dpr
          )

        atlasCtx.save()
        atlasCtx.beginPath()
        atlasCtx.rect(x0, y0, x1 - x0, y1 - y0)
        atlasCtx.clip()
        atlasCtx.drawImage(region.image, x0, y0, x1 - x0, y1 - y0)
        atlasCtx.restore()
      }

      atlasCtx.setTransform(dpr, 0, 0, dpr, ATLAS_PADDING_PX, ATLAS_PADDING_PX)
      const atlasTransform = {
        centerX: cx,
        viewLeft: expandedViewLeft,
        viewTop: expandedViewTop,
        scale,
      }
      if (landGrid) {
        drawLandPlotGrid(atlasCtx, expandedViewWorldSize, atlasTransform)
      }
      drawHouseMapFootprints(atlasCtx, houses, atlasTransform)

      ctx.clearRect(0, 0, cw, ch)
      ctx.fillStyle = OUT_OF_WORLD_OCEAN
      ctx.fillRect(0, 0, cw, ch)
      ctx.save()
      ctx.translate(cw / 2, ch / 2)
      ctx.rotate(MAP_ROTATE_ANGLE)
      ctx.translate(-cw / 2, -ch / 2)
      ctx.drawImage(
        atlas,
        (expandedViewLeft - viewLeft) * scale - ATLAS_PADDING_PX / dpr,
        (expandedViewTop - viewTop) * scale - ATLAS_PADDING_PX / dpr,
        atlas.width / dpr,
        atlas.height / dpr
      )
      ctx.restore()
      renderedView = {
        camX: cx,
        camZ: cz,
        zoomSpan: span,
        width: cw,
        height: ch,
      }
    })
  })

  // --- Place-name label overlay (HTML layer, not burned into the canvas) ---
  interface PlacedLabel {
    key: string
    name: string
    kind: LabelKind
    left: number
    top: number
    area: boolean
    textOffsetX: number
    textOffsetY: number
    textVisible: boolean
  }

  // World → overlay coords: unwrap x toward the camera (a point just across
  // the world seam renders near the edge instead of a full wrap away), scale
  // around the view center, then the same -45° rotation the canvas applies
  // (ctx.rotate(MAP_ROTATE_ANGLE)).
  function worldToScreen(x: number, z: number, view: RenderedView) {
    x = unwrapWorldXNear(view.camX, x)
    const scale =
      Math.min(view.width, view.height) / (view.zoomSpan * REGION_PX)
    const lx = (x - (view.camX - view.width / scale / 2)) * scale
    const ly = (z - (view.camZ - view.height / scale / 2)) * scale
    const ox = lx - view.width / 2
    const oy = ly - view.height / 2
    return {
      left: ox * COS_R - oy * SIN_R + view.width / 2,
      top: ox * SIN_R + oy * COS_R + view.height / 2,
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
        id: e.id,
        key: `dungeon:${e.id}`,
        name: e.name,
        kind: 'dungeon' as const,
        x: e.x,
        z: e.z,
      })
    )
    return [...MAP_LABELS, ...dungeons]
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
    const view = renderedView
    if (!roster || !view) return []

    // Join against the roster: a member who left since the last push (or an
    // id the roster never knew) must not draw a ghost.
    const names = new Map(roster.members.map((m) => [m.id, m.name]))
    const out: PartyMarker[] = []
    for (const pos of positions) {
      const name = names.get(pos.id)
      if (!name) continue
      const p = worldToScreen(pos.x, pos.z, view)
      if (!onScreen(p, view.width, view.height, 40)) continue
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

  let visibleLabels = $derived.by<PlacedLabel[]>(() => {
    const view = renderedView
    if (!view) return []
    const cw = view.width
    const ch = view.height

    const margin = 80 // keep labels whose anchor is just off-edge
    const inputs: { label: MapLabel; anchor: { x: number; y: number } }[] = []
    for (const label of mapLabels) {
      const p = worldToScreen(label.x, label.z, view)
      if (!onScreen(p, cw, ch, margin)) continue
      inputs.push({
        label,
        anchor: { x: p.left, y: p.top },
      })
    }

    const viewport = { left: 0, top: 0, right: cw, bottom: ch }
    const reservedBounds: ScreenRect[] =
      getMapFrameCornerReservedBounds(viewport)
    const player = worldToScreen(playerX, playerZ, view)
    if (onScreen(player, cw, ch, 20)) {
      reservedBounds.push({
        left: player.left - 12,
        top: player.top - 12,
        right: player.left + 12,
        bottom: player.top + 12,
      })
    }

    for (const marker of partyMarkers) {
      reservedBounds.push({
        left: marker.left - 9,
        top: marker.top - 13,
        right: marker.left + 16 + marker.name.length * 7,
        bottom: marker.top + 13,
      })
    }

    return layoutMapLabels(inputs, {
      zoomSpan: view.zoomSpan,
      areaZoomSpan: mobileMapBudget
        ? (view.zoomSpan * MOBILE_AREA_ZOOM_REFERENCE) / maxZoomSpan
        : view.zoomSpan,
      viewport,
      reservedBounds,
      collisionPadding: 5,
      edgePadding: 16,
      markerGap: 11,
    }).map(({ label, anchor, textOffset, textVisible }) => ({
      key: label.key,
      name: label.name,
      kind: label.kind,
      left: anchor.x,
      top: anchor.y,
      area: isFixedMapLabel(label.kind),
      textOffsetX: textOffset.x,
      textOffsetY: textOffset.y,
      textVisible,
    }))
  })

  let selfMarker = $derived.by<{
    left: number
    top: number
    angle: number
  } | null>(() => {
    const view = renderedView
    if (!view) return null
    const p = worldToScreen(playerX, playerZ, view)
    if (!onScreen(p, view.width, view.height, 20)) return null
    return {
      ...p,
      angle: headingToMapAngle(playerHeading),
    }
  })

  // --- Zoom controls ---
  function zoomIn() {
    useDefaultZoomOnOpen = false
    zoomSpan = Math.max(MIN_ZOOM, zoomSpan - 1)
  }

  function zoomOut() {
    useDefaultZoomOnOpen = false
    zoomSpan = Math.min(maxZoomSpan, zoomSpan + 1)
  }

  function zoomReset() {
    zoomSpan = defaultZoomSpan
    useDefaultZoomOnOpen = true
    savedZoom = null
  }

  function resetCamera() {
    camX = playerX
    camZ = playerZ
    followPlayerOnOpen = true
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
      followPlayerOnOpen = false
    }
    const delta = screenDeltaToWorld(rawDx / scale, rawDz / scale)
    camX = dragStartCamX - delta.x
    camZ = dragStartCamZ - delta.z
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
      const saved = persistWorldMapView(
        { camX, camZ, zoomSpan },
        { followPlayerOnOpen, useDefaultZoomOnOpen }
      )
      savedCamX = saved.camX
      savedCamZ = saved.camZ
      savedZoom = saved.zoomSpan
    }
  })

  // --- Actions ---
  function close() {
    if (mobileMapBudget) {
      renderGeneration++
      regionImages.flush()
    }
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
    const teleportRequested = event.ctrlKey && $isAdminUser
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
    // Invert worldToScreen against the view actually on screen, so a click
    // during an in-flight pan/zoom lands where the admin sees, not where the
    // camera already moved.
    const view = renderedView
    if (!containerEl || !view) return

    const rect = containerEl.getBoundingClientRect()
    const pixelX = clientX - rect.left
    const pixelY = clientY - rect.top

    const viewSize = view.zoomSpan * REGION_PX
    const canvasSize = Math.min(view.width, view.height)
    const scale = canvasSize / viewSize

    const delta = screenDeltaToWorld(
      (pixelX - view.width / 2) / scale,
      (pixelY - view.height / 2) / scale
    )
    teleportLocalPlayer(view.camX + delta.x, 0, view.camZ + delta.z)
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
    style="--wm-frame: url({assetUrl(
      '/textures/ui/world-map/ornate-frame.webp'
    )}); --wm-wood: url({assetUrl('/textures/ui/world-map/dark-wood.webp')})"
    role="dialog"
    aria-modal="true"
    aria-labelledby="world-map-title"
    tabindex="-1"
  >
    <div class="header">
      <h2 id="world-map-title">World Map</h2>
      <div class="controls">
        <button
          type="button"
          class="ctrl-btn symbol-btn"
          onclick={zoomIn}
          title="Zoom In"
          aria-label="Zoom in">+</button
        >
        <button
          type="button"
          class="ctrl-btn symbol-btn"
          onclick={zoomOut}
          title="Zoom Out"
          aria-label="Zoom out">&minus;</button
        >
        <button
          type="button"
          class="ctrl-btn reset-btn"
          onclick={zoomReset}
          title="Reset Zoom"
          aria-label="Reset zoom">Reset</button
        >
        <button
          type="button"
          class="ctrl-btn center-btn"
          onclick={resetCamera}
          title="Center on Player"
          aria-label="Center on player"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="12" r="6.5"></circle>
            <path d="M12 2.5v4M12 17.5v4M2.5 12h4M17.5 12h4"></path>
            <circle cx="12" cy="12" r="1.5" class="center-dot"></circle>
          </svg></button
        >
      </div>
      <button
        type="button"
        class="close-btn"
        onclick={close}
        title="Close"
        aria-label="Close world map">&times;</button
      >
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
      <canvas bind:this={canvasEl} class="map-canvas"></canvas>
      <div class="label-layer">
        {#each visibleLabels as label (label.key)}
          <div
            class="map-label {label.kind}"
            class:area={label.area}
            style="left: {label.left}px; top: {label.top}px; --label-text-x: {label.textOffsetX}px; --label-text-y: {label.textOffsetY}px;"
          >
            {#if label.kind === 'capital' || label.kind === 'city' || label.kind === 'town'}
              <svg
                class="marker crest-marker"
                viewBox="0 0 18 24"
                aria-hidden="true"
              >
                <path
                  class="crest-shield"
                  d="M2.25 2.25h13.5v9.2c0 5.15-2.95 8.45-6.75 10.3-3.8-1.85-6.75-5.15-6.75-10.3z"
                ></path>
                <path
                  class="crest-sigil"
                  d="M5.3 7.1h7.4M6.4 7.1v4.25h5.2V7.1M7.2 11.35v3.9M10.8 11.35v3.9M5.8 15.25h6.4"
                ></path>
              </svg>
            {:else if !label.area}
              <span class="marker"></span>
            {/if}
            {#if label.textVisible}
              <span class="text">{label.name}</span>
            {/if}
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
          <SelfMarker
            left={selfMarker.left}
            top={selfMarker.top}
            angle={selfMarker.angle}
          />
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
    background:
      radial-gradient(
        circle at 50% 42%,
        rgba(35, 47, 38, 0.16),
        transparent 58%
      ),
      rgba(2, 4, 3, 0.74);
    backdrop-filter: blur(3px);
    z-index: 30;
  }

  .dialog {
    --wm-night: #080b09;
    --wm-paper: #eee2c1;
    --wm-gold: #c49b4b;
    --wm-gold-hi: #f0ce78;
    --wm-brass: #76572b;
    --wm-serif:
      'Palatino Linotype', Palatino, 'Book Antiqua', Georgia, 'Times New Roman',
      'Noto Serif KR', AppleMyungjo, Batang, serif;
    position: relative;
    isolation: isolate;
    width: min(80vw, 80dvh, 800px);
    height: min(80vw, 80dvh, 800px);
    display: flex;
    flex-direction: column;
    border: 1px solid #38270e;
    border-radius: 5px;
    background: var(--wm-night);
    color: var(--wm-paper);
    box-shadow:
      0 26px 80px rgba(0, 0, 0, 0.72),
      0 0 0 1px #160f07,
      0 0 18px rgba(196, 155, 75, 0.18);
    overflow: hidden;
  }

  .dialog::before {
    content: '';
    position: absolute;
    inset: 0;
    z-index: 10;
    background: var(--wm-frame) center / 100% 100% no-repeat;
    pointer-events: none;
  }

  .dialog.mobile-map-budget {
    width: min(
      calc(
        100vw - 16px - env(safe-area-inset-left) - env(safe-area-inset-right)
      ),
      calc(
        100dvh - 96px - env(safe-area-inset-top) - env(safe-area-inset-bottom)
      ),
      440px
    );
    height: auto;
    aspect-ratio: 1;
  }

  .header {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    flex: 0 0 52px;
    /* The ornate frame overlay hides the top 18/1254 of the square dialog, so
       matching top padding keeps the content on the visible wood bar's center. */
    padding: calc(18 / 1254 * 100%) max(30px, 8%) 0;
    border-bottom: 1px solid var(--wm-brass);
    background:
      linear-gradient(180deg, rgba(55, 38, 20, 0.2), rgba(7, 7, 5, 0.72)),
      var(--wm-wood) center 44% / cover;
    box-shadow:
      inset 0 -1px rgba(240, 206, 120, 0.22),
      0 3px 12px rgba(0, 0, 0, 0.38);
  }

  .header h2,
  .controls,
  .close-btn {
    position: relative;
    z-index: 11;
  }

  .header h2 {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--wm-paper);
    font-family: var(--wm-serif);
    font-size: 20px;
    font-weight: 700;
    letter-spacing: 0.06em;
    line-height: 1;
    text-overflow: ellipsis;
    text-shadow: 0 2px 3px #080604;
    white-space: nowrap;
  }

  .controls {
    grid-column: 2;
    display: flex;
    gap: 6px;
  }

  .ctrl-btn {
    min-width: 34px;
    height: 34px;
    padding: 0 9px;
    border: 1px solid var(--wm-brass);
    border-radius: 4px;
    background:
      linear-gradient(180deg, rgba(57, 48, 31, 0.68), rgba(13, 13, 10, 0.9)),
      var(--wm-wood) center / 220px 220px;
    box-shadow:
      inset 0 0 0 1px rgba(240, 206, 120, 0.12),
      0 2px 4px rgba(0, 0, 0, 0.36);
    color: #d8ccb0;
    font-family: var(--wm-serif);
    font-size: 15px;
    cursor: pointer;
    line-height: 1;
    transition:
      border-color 120ms ease,
      background 120ms ease,
      color 120ms ease;
  }

  .ctrl-btn:hover {
    border-color: var(--wm-gold-hi);
    background:
      linear-gradient(180deg, rgba(96, 74, 39, 0.62), rgba(27, 22, 14, 0.92)),
      var(--wm-wood) center / 220px 220px;
    color: #fff0c8;
  }

  .ctrl-btn.symbol-btn {
    padding: 0;
    font-size: 22px;
    font-weight: 400;
  }

  .ctrl-btn.center-btn {
    display: grid;
    place-items: center;
    padding: 0;
  }

  .center-btn svg {
    width: 19px;
    height: 19px;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-width: 1.5;
  }

  .center-btn .center-dot {
    fill: currentColor;
    stroke: none;
  }

  .close-btn {
    grid-column: 3;
    justify-self: end;
    background: none;
    border: none;
    color: #c7a866;
    font-family: var(--wm-serif);
    font-size: 27px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #ffe8ad;
  }

  .ctrl-btn:focus-visible,
  .close-btn:focus-visible {
    outline: 2px solid var(--wm-gold-hi);
    outline-offset: 2px;
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

  .map-container::before,
  .map-container::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .map-container::before {
    z-index: 3;
    box-shadow: inset 0 0 0 1px rgba(240, 206, 120, 0.18);
  }

  .map-container::after {
    z-index: 4;
    background: radial-gradient(
      ellipse at center,
      transparent 67%,
      rgba(7, 8, 5, 0.07) 83%,
      rgba(5, 6, 4, 0.22) 100%
    );
    box-shadow: inset 0 0 32px rgba(5, 6, 4, 0.18);
  }

  .map-container.dragging {
    cursor: grabbing;
  }

  .map-canvas {
    position: absolute;
    inset: 0;
    display: block;
    width: 100%;
    height: 100%;
    background: #0a1417;
  }

  .label-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    z-index: 2;
    --label-halo:
      0 1px 1px rgba(24, 14, 6, 0.98), 0 0 3px rgba(12, 8, 4, 0.9),
      0 2px 6px rgba(0, 0, 0, 0.66);
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
    font-family: var(--wm-serif);
    transform: translate(
      calc(-50% + var(--label-text-x)),
      calc(-50% + var(--label-text-y))
    );
    text-shadow: var(--label-halo);
  }

  .map-label.area .text {
    text-align: center;
  }

  .map-label.continent .text {
    font-size: 32px;
    font-weight: 600;
    letter-spacing: 8px;
    color: #f0dfb6;
    text-shadow:
      0 1px 0 #5b3d19,
      0 0 3px rgba(8, 6, 3, 0.96),
      0 3px 7px rgba(0, 0, 0, 0.78);
  }

  .map-label.sea .text {
    font-size: 21px;
    font-style: italic;
    font-weight: 600;
    letter-spacing: 3px;
    color: #abcbd5;
    text-shadow:
      0 1px 0 rgba(133, 179, 194, 0.24),
      0 0 3px rgba(3, 19, 30, 0.96),
      0 3px 7px rgba(0, 0, 0, 0.78);
  }

  .map-label.island .text {
    font-size: 16px;
    font-weight: 600;
    font-style: italic;
    letter-spacing: 0.6px;
    color: #ddd7bc;
  }

  .map-label.capital .text {
    font-size: 17px;
    font-weight: 700;
    letter-spacing: 0.3px;
    color: #fff0c8;
  }

  .map-label.city .text {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: 0.2px;
    color: #efe5c9;
  }

  .map-label.town .text {
    font-size: 15px;
    font-weight: 600;
    letter-spacing: 0.2px;
    color: #ddd2b1;
  }

  .map-label .crest-marker {
    overflow: visible;
    border-radius: 0;
    filter: drop-shadow(0 2px 2px rgba(0, 0, 0, 0.82));
  }

  .map-label.capital .crest-marker {
    width: 15px;
    height: 20px;
  }

  .map-label.city .crest-marker {
    width: 14px;
    height: 19px;
  }

  .map-label.town .crest-marker {
    width: 13px;
    height: 18px;
  }

  .crest-shield {
    fill: #17170f;
    stroke: #d4a536;
    stroke-linejoin: round;
    stroke-width: 1.6;
  }

  .crest-sigil {
    fill: none;
    stroke: #f2ca62;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-width: 1.25;
  }

  .map-label.city .crest-shield {
    stroke: #c59632;
  }

  .map-label.town .crest-shield {
    fill: #1b2117;
    stroke: #bd9132;
  }

  .map-label.town .crest-sigil {
    stroke: #dfb84d;
  }

  .map-label.dungeon .text {
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.2px;
    color: #d7b3a5;
  }

  .map-label.dungeon .marker {
    width: 9px;
    height: 9px;
    border-radius: 0;
    transform: translate(-50%, -50%) rotate(45deg);
    background: #29272c;
    border: 2px solid #985246;
    box-shadow: 0 1px 3px rgba(62, 22, 18, 0.68);
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

  @media (max-width: 520px) {
    .dialog {
      width: calc(100vw - 12px);
      height: min(calc(100dvh - 72px), calc(100vw - 12px));
    }

    .dialog::before {
      background-size: 100% 100%;
    }

    .header {
      flex-basis: 48px;
      padding: 0 max(24px, 8%);
    }

    .header h2 {
      font-size: 15px;
      letter-spacing: 0.07em;
    }

    .controls {
      gap: 3px;
    }

    .ctrl-btn {
      min-width: 29px;
      height: 31px;
      padding: 0 6px;
      font-size: 13px;
    }

    .close-btn {
      font-size: 20px;
      padding-right: 1px;
    }
  }
</style>

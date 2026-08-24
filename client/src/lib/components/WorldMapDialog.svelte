<script module lang="ts">
  import {
    MAP_LABELS as MAP_LABEL_DEFS,
    type MapLabelDef,
    type MapLabelKind,
  } from '../data/mapLabels'
  import { RegionImageCache } from '../terrain/regionImageCache'
  import {
    layoutMapLabels,
    type ScreenRect,
  } from '../utils/worldMapLabelLayout'

  const REGION_SIZE = 16
  const TILE_DIM = 64
  const REGION_PX = REGION_SIZE * TILE_DIM // 1024
  const WORLD_MIN_REGION_Z = -16
  const WORLD_MAX_REGION_Z = 15

  const MIN_ZOOM = 1
  const DEFAULT_ZOOM = 8

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
  import { partyRoster, partyPositions } from '../stores/partyStore'
  import { worldMapVisible } from '../stores/debugStore'
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
  import { teleportLocalPlayer } from '../utils/teleport'

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
    const dpr = Math.min(
      window.devicePixelRatio || 1,
      graphicsPreset.pixelRatioCap
    )
    const gen = ++renderGeneration

    const backingW = Math.max(1, Math.round(cw * dpr))
    const backingH = Math.max(1, Math.round(ch * dpr))
    if (canvasEl.width !== backingW) canvasEl.width = backingW
    if (canvasEl.height !== backingH) canvasEl.height = backingH
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
    const sourceSize =
      projectedRegionPx <= 128
        ? 128
        : projectedRegionPx <= 256
          ? 256
          : projectedRegionPx <= 512
            ? 512
            : 1024
    if (!mobileMapBudget) {
      regionImages.limit =
        sourceSize === 128
          ? Math.max(imageCacheLimit, 1024)
          : sourceSize === 256
            ? Math.max(imageCacheLimit, 512)
            : imageCacheLimit
    }

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
      if (rz < WORLD_MIN_REGION_Z || rz > WORLD_MAX_REGION_Z) continue
      for (let rx = expRegionMinRx; rx <= expRegionMaxRx; rx++) {
        // Region world origin
        const regionWorldX = rx * REGION_PX - TILE_DIM / 2
        const regionWorldZ = rz * REGION_PX - TILE_DIM / 2

        // Canvas position (before rotation, relative to view center)
        const drawX = Math.floor((regionWorldX - viewLeft) * scale)
        const drawY = Math.floor((regionWorldZ - viewTop) * scale)
        const drawSize = Math.ceil(REGION_PX * scale)

        promises.push(
          regionImages.load(rx, rz, mmVer, sourceSize).then((img) => {
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
    area: boolean
    textOffsetX: number
    textOffsetY: number
    textVisible: boolean
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

  let visibleLabels = $derived.by<PlacedLabel[]>(() => {
    const cw = containerW
    const ch = containerH
    if (cw <= 0 || ch <= 0) return []

    const margin = 80 // keep labels whose anchor is just off-edge
    const inputs: { label: MapLabel; anchor: { x: number; y: number } }[] = []
    for (const label of mapLabels) {
      const p = worldToScreen(label.x, label.z, cw, ch)
      if (!onScreen(p, cw, ch, margin)) continue
      inputs.push({
        label,
        anchor: { x: p.left, y: p.top },
      })
    }

    const reservedBounds: ScreenRect[] = []
    const player = worldToScreen(playerX, playerZ, cw, ch)
    if (onScreen(player, cw, ch, 20)) {
      reservedBounds.push({
        left: player.left - 12,
        top: player.top - 12,
        right: player.left + 12,
        bottom: player.top + 12,
      })
    }

    const roster = $partyRoster
    if (roster) {
      const names = new Map(
        roster.members.map((member) => [member.id, member.name])
      )
      for (const position of $partyPositions) {
        const name = names.get(position.id)
        if (!name) continue
        const p = worldToScreen(position.x, position.z, cw, ch)
        if (!onScreen(p, cw, ch, 40)) continue
        reservedBounds.push({
          left: p.left - 9,
          top: p.top - 13,
          right: p.left + 16 + name.length * 7,
          bottom: p.top + 13,
        })
      }
    }

    return layoutMapLabels(inputs, {
      zoomSpan,
      areaZoomSpan: mobileMapBudget
        ? (zoomSpan * DEFAULT_ZOOM) / maxZoomSpan
        : zoomSpan,
      viewport: { left: 0, top: 0, right: cw, bottom: ch },
      reservedBounds,
      collisionPadding: 5,
      edgePadding: 8,
      markerGap: 11,
    }).map(({ label, anchor, textOffset, textVisible }) => ({
      key: label.key,
      name: label.name,
      kind: label.kind,
      left: anchor.x,
      top: anchor.y,
      area:
        label.kind === 'continent' ||
        label.kind === 'sea' ||
        label.kind === 'island',
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

    // Join against the roster: a member who left since the last push (or an
    // id the roster never knew) must not draw a ghost.
    const names = new Map(roster.members.map((m) => [m.id, m.name]))
    const out: PartyMarker[] = []
    for (const pos of positions) {
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
    const worldX = camX + (sx * cosA - sz * sinA)
    const worldZ = camZ + (sx * sinA + sz * cosA)

    teleportLocalPlayer(worldX, 0, worldZ)
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
      <canvas bind:this={canvasEl} class="map-canvas"></canvas>
      <div class="label-layer">
        {#each visibleLabels as label (label.key)}
          <div
            class="map-label {label.kind}"
            class:area={label.area}
            style="left: {label.left}px; top: {label.top}px; --label-text-x: {label.textOffsetX}px; --label-text-y: {label.textOffsetY}px;"
          >
            {#if !label.area}
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
    position: relative;
    isolation: isolate;
    width: min(80vw, 800px);
    height: min(80vh, 800px);
    display: flex;
    flex-direction: column;
    border-radius: 8px;
    border: 1px solid var(--wm-brass);
    background: var(--wm-night);
    color: var(--wm-paper);
    box-shadow:
      0 26px 80px rgba(0, 0, 0, 0.72),
      0 0 0 1px #21170b,
      inset 0 0 0 1px rgba(240, 206, 120, 0.16);
    overflow: hidden;
  }

  .dialog::before {
    content: '';
    position: absolute;
    inset: 5px;
    z-index: 10;
    border: 1px solid rgba(240, 206, 120, 0.2);
    border-radius: 4px;
    background:
      linear-gradient(90deg, var(--wm-gold), transparent) left top / 28px 1px
        no-repeat,
      linear-gradient(180deg, var(--wm-gold), transparent) left top / 1px 28px
        no-repeat,
      linear-gradient(-90deg, var(--wm-gold), transparent) right top / 28px 1px
        no-repeat,
      linear-gradient(180deg, var(--wm-gold), transparent) right top / 1px 28px
        no-repeat,
      linear-gradient(90deg, var(--wm-gold), transparent) left bottom / 28px 1px
        no-repeat,
      linear-gradient(0deg, var(--wm-gold), transparent) left bottom / 1px 28px
        no-repeat,
      linear-gradient(-90deg, var(--wm-gold), transparent) right bottom / 28px
        1px no-repeat,
      linear-gradient(0deg, var(--wm-gold), transparent) right bottom / 1px 28px
        no-repeat;
    pointer-events: none;
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
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    flex: 0 0 52px;
    padding: 0 15px;
    border-bottom: 1px solid var(--wm-brass);
    background:
      linear-gradient(180deg, rgba(72, 57, 34, 0.62), rgba(17, 20, 15, 0.96)),
      repeating-linear-gradient(94deg, #1c170f 0 18px, #241c11 20px 38px);
    box-shadow:
      inset 0 -1px rgba(240, 206, 120, 0.12),
      0 3px 12px rgba(0, 0, 0, 0.38);
  }

  .header::after {
    content: '';
    position: absolute;
    left: 50%;
    bottom: -4px;
    width: 7px;
    height: 7px;
    border: 1px solid var(--wm-brass);
    background: #171b14;
    transform: translateX(-50%) rotate(45deg);
  }

  .header h2 {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--wm-paper);
    font-family:
      Georgia, 'Times New Roman', 'Noto Serif KR', AppleMyungjo, Batang, serif;
    font-size: 17px;
    font-weight: 700;
    letter-spacing: 0.13em;
    line-height: 1;
    text-overflow: ellipsis;
    text-shadow: 0 2px 3px #080604;
    white-space: nowrap;
  }

  .controls {
    grid-column: 2;
    display: flex;
    gap: 5px;
  }

  .ctrl-btn {
    min-width: 32px;
    height: 31px;
    padding: 0 8px;
    border: 1px solid var(--wm-brass);
    border-radius: 4px;
    background: linear-gradient(180deg, #273027, #111711);
    box-shadow:
      inset 0 0 0 1px rgba(240, 206, 120, 0.08),
      0 2px 4px rgba(0, 0, 0, 0.36);
    color: #d8ccb0;
    font-family:
      Georgia, 'Times New Roman', 'Noto Serif KR', AppleMyungjo, Batang, serif;
    font-size: 14px;
    cursor: pointer;
    line-height: 1;
    transition:
      border-color 120ms ease,
      background 120ms ease,
      color 120ms ease;
  }

  .ctrl-btn:hover {
    border-color: var(--wm-gold-hi);
    background: linear-gradient(180deg, #364133, #192119);
    color: #fff0c8;
  }

  .ctrl-btn.active {
    border-color: var(--wm-gold-hi);
    background: linear-gradient(180deg, #574626, #282113);
    color: #ffe39b;
  }

  .close-btn {
    grid-column: 3;
    justify-self: end;
    background: none;
    border: none;
    color: #b6a98d;
    font-size: 22px;
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
      transparent 62%,
      rgba(7, 8, 5, 0.12) 78%,
      rgba(5, 6, 4, 0.44) 100%
    );
    box-shadow: inset 0 0 44px rgba(5, 6, 4, 0.34);
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
    font-family:
      Georgia, 'Times New Roman', 'Noto Serif KR', AppleMyungjo, Batang, serif;
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
    font-size: 22px;
    font-weight: 600;
    letter-spacing: 4px;
    color: #eee1bd;
    text-shadow:
      0 1px 0 #4a3115,
      0 0 4px rgba(8, 6, 3, 0.92),
      0 3px 8px rgba(0, 0, 0, 0.72);
  }

  .map-label.sea .text {
    font-size: 15px;
    font-style: italic;
    font-weight: 600;
    letter-spacing: 2px;
    color: #a8cbd4;
  }

  .map-label.island .text {
    font-size: 13px;
    font-weight: 600;
    font-style: italic;
    letter-spacing: 0.4px;
    color: #d8d4b8;
  }

  .map-label.capital .text {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: 0.3px;
    color: #fff0c8;
  }

  .map-label.city .text {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.2px;
    color: #efe5c9;
  }

  .map-label.town .text {
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.2px;
    color: #ddd2b1;
  }

  .map-label.capital .marker {
    width: 12px;
    height: 12px;
    border: 2px solid #271b0b;
    border-radius: 2px;
    background: linear-gradient(135deg, #ffe395, #b67b21);
    box-shadow:
      0 0 0 2px rgba(236, 193, 91, 0.68),
      0 2px 5px rgba(0, 0, 0, 0.7);
    transform: translate(-50%, -50%) rotate(45deg);
  }

  .map-label.city .marker {
    width: 9px;
    height: 9px;
    background: #d7a947;
    border: 2px solid #281d0e;
    box-shadow: 0 0 0 1px rgba(255, 225, 148, 0.42);
  }

  .map-label.town .marker {
    width: 8px;
    height: 8px;
    background: #1c251b;
    border: 2px solid #c59b4a;
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

  @media (max-width: 520px) {
    .dialog {
      width: calc(100vw - 12px);
      height: min(calc(100dvh - 72px), calc(100vw - 12px));
    }

    .dialog::before {
      inset: 3px;
    }

    .header {
      flex-basis: 48px;
      padding: 0 10px;
    }

    .header h2 {
      font-size: 14px;
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

    .map-label.continent .text {
      font-size: 19px;
      letter-spacing: 3px;
    }
  }
</style>

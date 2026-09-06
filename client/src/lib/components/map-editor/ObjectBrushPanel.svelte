<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { get } from 'svelte/store'
  import {
    objectCatalog,
    selectedObjectType,
    objectRotation,
    currentObjectData,
    selectedObjectPlacementId,
    objectSubTool,
    editorHeightManager,
    editorGrassDataManager,
  } from '../../stores/editorStore'
  import type {
    ObjectDef,
    ObjectPlacement,
    ObjectRegionData,
    ObjectSubTool,
  } from '../../stores/editorStore'
  import { objectManager } from '../../managers/objectManager'
  import { furnitureManager } from '../../managers/furnitureManager'
  import {
    deleteSelectedPlacement,
    duplicateSelectedPlacement,
  } from './object-edit'
  import {
    rotatedRectAabb,
    type FootprintRect,
  } from '../../utils/objectFootprint'
  import {
    playerVisualFloorLevel,
    playerInsideHouseId,
  } from '../../stores/housingStore'
  import { currentEditorRegion } from '../../stores/editorStore'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'
  import type { TerrainGrassDataManager } from '../../managers/terrainGrassDataManager'

  let catalog = $state<ObjectDef[]>([])
  let selected = $state<string | null>(null)
  let rotation = $state(0)
  let placements = $state<ObjectPlacement[]>([])
  let selectedPlacementId = $state<number | null>(null)
  /** Live text-edit buffer for the selected placement; flushed on deselect/blur
   *  so a canvas click (which doesn't blur the textarea) can't drop it. */
  let textDraft = $state('')
  let subTool = $state<ObjectSubTool>('place')
  let floor = $state(0)
  let insideHouseId = $state<string | null>(null)
  /** Anchors slider range so it doesn't drift while dragging. */
  let baseY = $state<number | null>(null)
  let baseX = $state<number | null>(null)
  let baseZ = $state<number | null>(null)
  let heightManager = $state<TerrainHeightManager | null>(null)
  let grassManager = $state<TerrainGrassDataManager | null>(null)
  let flattening = $state(false)

  const Y_RANGE = 5
  const XZ_RANGE = 5
  /** Tight blend so flattening doesn't bleed into surrounding terrain (e.g. river banks). */
  const FLATTEN_BLEND_RADIUS = 2

  const unsubs = [
    objectCatalog.subscribe((v) => (catalog = v)),
    selectedObjectType.subscribe((v) => (selected = v)),
    objectRotation.subscribe((v) => (rotation = v)),
    currentObjectData.subscribe((v) => (placements = v.placements)),
    selectedObjectPlacementId.subscribe((id) => {
      // Persist any pending text edit on the previously-selected placement
      // before switching, so deselecting (e.g. clicking the canvas) keeps it.
      flushTextDraft()
      selectedPlacementId = id
      if (id === null) {
        baseY = null
        baseX = null
        baseZ = null
        textDraft = ''
      } else {
        const p = get(currentObjectData).placements.find((p) => p.id === id)
        baseY = p?.y ?? null
        baseX = p?.x ?? null
        baseZ = p?.z ?? null
        textDraft = p?.text ?? ''
      }
    }),
    objectSubTool.subscribe((v) => (subTool = v)),
    playerVisualFloorLevel.subscribe((v) => (floor = v)),
    playerInsideHouseId.subscribe((v) => (insideHouseId = v)),
    editorHeightManager.subscribe((v) => (heightManager = v)),
    editorGrassDataManager.subscribe((v) => (grassManager = v)),
  ]
  onDestroy(() => {
    flushTextDraft()
    if (saveTimer !== null) flushPendingSave()
    unsubs.forEach((u) => u())
  })

  onMount(async () => {
    if (get(objectCatalog).length > 0) return
    const list = await objectManager.fetchCatalog()
    objectCatalog.set(list)
  })

  function selectType(id: string) {
    selectedObjectType.set(id)
    objectSubTool.set('place')
    selectedObjectPlacementId.set(null)
  }

  function setSubTool(tool: ObjectSubTool) {
    objectSubTool.set(tool)
    if (tool === 'place') {
      selectedObjectPlacementId.set(null)
    }
  }

  /** Patch a field (or fields) on the selected placement in the store. Used by
   *  the Y and Rot sliders' live-drag (oninput) path. Returns the updated region
   *  data, or null if nothing is selected. */
  function applyPatch(
    patch: Partial<ObjectPlacement>
  ): ObjectRegionData | null {
    if (selectedPlacementId === null) return null
    const data = get(currentObjectData)
    const updated: ObjectRegionData = {
      placements: data.placements.map((p) => {
        if (p.id !== selectedPlacementId) return p
        const merged = { ...p, ...patch }
        // Solid furniture stays on 90° yaw so its footprint lands on whole cells.
        if (patch.rotation !== undefined) {
          merged.rotation = furnitureManager.snapRotation(
            merged.type,
            merged.rotation
          )
        }
        return merged
      }),
    }
    currentObjectData.set(updated)
    return updated
  }

  /** applyPatch + persist to disk. Used by the sliders' commit (onchange) path. */
  function commitPatch(patch: Partial<ObjectPlacement>) {
    const updated = applyPatch(patch)
    if (!updated) return
    const region = get(currentEditorRegion)
    if (region) {
      objectManager.saveObject(region.rx, region.rz, updated)
    }
  }

  let saveTimer: ReturnType<typeof setTimeout> | null = null
  /** Region + data snapshot captured when a debounced save is scheduled, so the
   *  edit is persisted to the region it was made in even if the editor switches
   *  regions (which replaces currentObjectData) before the timer fires. */
  let pendingSaveRegion: { rx: number; rz: number } | null = null
  let pendingSaveData: ObjectRegionData | null = null

  /** Persist the current region, coalescing rapid edits (e.g. wheel notches)
   *  into a single disk write instead of one PUT per change. */
  function scheduleSave() {
    // applyPatch has already published a fresh immutable snapshot to the store,
    // so capturing it here pins the edit even if the store is later replaced.
    pendingSaveRegion = get(currentEditorRegion)
    pendingSaveData = get(currentObjectData)
    if (saveTimer !== null) clearTimeout(saveTimer)
    saveTimer = setTimeout(flushPendingSave, 250)
  }

  function flushPendingSave() {
    if (saveTimer !== null) {
      clearTimeout(saveTimer)
      saveTimer = null
    }
    if (pendingSaveRegion && pendingSaveData) {
      objectManager.saveObject(
        pendingSaveRegion.rx,
        pendingSaveRegion.rz,
        pendingSaveData
      )
    }
    pendingSaveRegion = null
    pendingSaveData = null
  }

  /** Nudge a numeric field by one `step` per wheel notch (up = increase).
   *  Applies to the store immediately for responsive feedback and debounces the
   *  disk save so scrolling doesn't fire a PUT per notch. Used by the X/Y/Z and
   *  Rot/RotX sliders so scrolling over them fine-tunes. Rotation fields wrap
   *  into [0, 360). */
  function wheelNudge(
    e: WheelEvent,
    field: 'x' | 'y' | 'z' | 'rotation' | 'rotationX',
    step: number
  ) {
    if (selectedPlacementId === null) return
    e.preventDefault()
    const p = get(currentObjectData).placements.find(
      (p) => p.id === selectedPlacementId
    )
    if (!p) return
    const dir = e.deltaY < 0 ? 1 : -1
    // Solid furniture yaw steps by 90° so each notch moves a full cell-aligned
    // turn (applyPatch would otherwise snap a sub-90° nudge back to the same value).
    const effStep =
      field === 'rotation' && furnitureManager.isSolid(p.type) ? 90 : step
    const cur = p[field] ?? 0
    let next = Math.round((cur + dir * effStep) / effStep) * effStep
    if (field === 'rotation' || field === 'rotationX') {
      next = ((next % 360) + 360) % 360
    }
    applyPatch({ [field]: next })
    // Re-anchor the X/Y/Z slider window so its thumb follows the wheeled value
    // instead of clamping at the edge of the range fixed at selection time.
    if (field === 'x') baseX = next
    else if (field === 'y') baseY = next
    else if (field === 'z') baseZ = next
    scheduleSave()
  }

  /** Persist the per-instance text buffer (signposts etc.) to the selected
   *  placement, saving only when it actually changed. Safe to call repeatedly. */
  function flushTextDraft() {
    if (selectedPlacementId === null) return
    const data = get(currentObjectData)
    const current = data.placements.find((p) => p.id === selectedPlacementId)
    if (!current) return
    const trimmed = textDraft.trim()
    const next = trimmed.length > 0 ? trimmed : undefined
    if ((current.text ?? undefined) === next) return
    const updated: ObjectRegionData = {
      placements: data.placements.map((p) =>
        p.id === selectedPlacementId ? { ...p, text: next } : p
      ),
    }
    currentObjectData.set(updated)
    const region = get(currentEditorRegion)
    if (region) {
      objectManager.saveObject(region.rx, region.rz, updated)
    }
  }

  async function flattenTerrain() {
    const p = selectedPlacement
    const hm = heightManager
    if (!p || !hm || flattening) return

    flattening = true
    try {
      const fp = await objectManager.fetchFootprint(p.type)
      if (!fp || fp.rects.length === 0) return

      const buryDepth =
        objectManager.getCatalogEntry(p.type)?.flattenBuryDepth ?? 0
      const targetY = p.y + fp.minLocalY + buryDepth
      // Flatten using the local rect + placement rotation so footprints rotated
      // off-axis (e.g. 45°) carve the right oriented region instead of bleeding
      // into the rotated rect's AABB corners.
      for (const r of fp.rects) {
        hm.flattenRotatedRect(
          p.x,
          p.z,
          p.rotation,
          r.minX,
          r.maxX,
          r.minZ,
          r.maxZ,
          targetY,
          FLATTEN_BLEND_RADIUS
        )
      }
      await hm.saveAllDirty()

      // World AABB of each rotated rect, used for grass-removal tile bucketing.
      // (removeGrassInRect is AABB-only — at non-90° rotations this over-clears
      // grass slightly into the AABB corners, which is acceptable.)
      const rot = (p.rotation * Math.PI) / 180
      const worldRects: FootprintRect[] = fp.rects.map((r) => {
        const a = rotatedRectAabb(r.minX, r.maxX, r.minZ, r.maxZ, rot)
        return {
          minX: p.x + a.minX,
          maxX: p.x + a.maxX,
          minZ: p.z + a.minZ,
          maxZ: p.z + a.maxZ,
        }
      })

      await grassManager?.removeGrassInRects(worldRects)
    } finally {
      flattening = false
    }
  }

  function formatPos(p: ObjectPlacement): string {
    return `${p.x.toFixed(1)}, ${p.y.toFixed(1)}, ${p.z.toFixed(1)}`
  }

  let selectedPlacement = $derived(
    placements.find((p) => p.id === selectedPlacementId) ?? null
  )

  let selectedDef = $derived(
    selectedPlacement
      ? (catalog.find((d) => d.id === selectedPlacement.type) ?? null)
      : null
  )
</script>

<div class="object-panel">
  <div class="panel-title">Object</div>

  <div class="sub-tools">
    <button
      class="sub-tool-btn"
      class:active={subTool === 'place'}
      onclick={() => setSubTool('place')}>Place</button
    >
    <button
      class="sub-tool-btn"
      class:active={subTool === 'select'}
      onclick={() => setSubTool('select')}>Select</button
    >
  </div>

  {#if subTool === 'place'}
    <div class="section-label">Catalog</div>
    <div class="object-list">
      {#each catalog as item (item.id)}
        <button
          class="object-item-btn"
          class:active={selected === item.id}
          onclick={() => selectType(item.id)}
        >
          <span class="item-name">{item.name}</span>
          <span class="item-action">{item.interaction}</span>
        </button>
      {/each}
    </div>

    {#if selected}
      <div class="section-label">Placement</div>
      <div class="rotation-display">
        <span class="rotation-value">{rotation}&deg;</span>
        <span class="rotation-hint">Press R to rotate</span>
      </div>
      <div class="rotation-display" style="margin-top: 2px">
        <span class="rotation-value"
          >{insideHouseId == null ? 'Outside' : `${floor + 1}F`}</span
        >
        <span class="rotation-hint">Follow player floor</span>
      </div>
    {:else}
      <div class="draw-hint">Select an object type to place</div>
    {/if}
  {:else}
    {#if selectedPlacement}
      <div class="section-label">Selected</div>
      <div class="selected-info">
        <div class="coord-row">
          <span class="info-label">Type:</span>
          <span class="info-value">{selectedPlacement.type}</span>
        </div>
        <div class="coord-row">
          <span class="info-label">ID:</span>
          <span class="info-value">{selectedPlacement.id}</span>
        </div>
        <div class="coord-row">
          <span class="info-label">Pos:</span>
          <span class="info-value">{formatPos(selectedPlacement)}</span>
        </div>
        {#snippet coordRow(
          label: string,
          field: 'x' | 'y' | 'z' | 'rotation' | 'rotationX',
          min: number,
          max: number,
          step: number,
          decimals: number,
          suffix: string
        )}
          <div class="coord-row">
            <span class="info-label">{label}:</span>
            <input
              class="y-slider"
              type="range"
              {min}
              {max}
              {step}
              value={selectedPlacement?.[field] ?? 0}
              oninput={(e) =>
                applyPatch({ [field]: parseFloat(e.currentTarget.value) })}
              onchange={(e) =>
                commitPatch({ [field]: parseFloat(e.currentTarget.value) })}
              onwheel={(e) => wheelNudge(e, field, step)}
            />
            <span class="y-value"
              >{(selectedPlacement?.[field] ?? 0).toFixed(
                decimals
              )}{suffix}</span
            >
          </div>
        {/snippet}
        {@render coordRow(
          'X',
          'x',
          (baseX ?? 0) - XZ_RANGE,
          (baseX ?? 0) + XZ_RANGE,
          0.05,
          2,
          ''
        )}
        {@render coordRow(
          'Y',
          'y',
          (baseY ?? 0) - Y_RANGE,
          (baseY ?? 0) + Y_RANGE,
          0.05,
          2,
          ''
        )}
        {@render coordRow(
          'Z',
          'z',
          (baseZ ?? 0) - XZ_RANGE,
          (baseZ ?? 0) + XZ_RANGE,
          0.05,
          2,
          ''
        )}
        {@render coordRow('Rot', 'rotation', 0, 360, 15, 0, '°')}
        <!-- Bridges derive their walkable deck from yaw only; a pitched bridge
             would render tilted while its collision/deck-Y stays flat. -->
        {#if selectedDef?.kind !== 'bridge'}
          {@render coordRow('RotX', 'rotationX', 0, 360, 15, 0, '°')}
        {/if}
        <div class="coord-row">
          <span class="info-label">Floor:</span>
          <span class="info-value">{selectedPlacement.floorLevel + 1}F</span>
        </div>
        {#if selectedDef?.procedural === 'shopSign'}
          <label class="text-field">
            <span class="info-label">Style:</span>
            <select
              class="text-input"
              value={selectedPlacement.type}
              onchange={(e) => {
                applyPatch({
                  type: e.currentTarget.value,
                  text: textDraft.trim() || undefined,
                })
                scheduleSave()
              }}
            >
              {#each catalog.filter((item) => item.procedural === 'shopSign') as sign (sign.id)}
                <option value={sign.id}>{sign.name}</option>
              {/each}
            </select>
          </label>
        {/if}
        {#if selectedDef?.textLabel}
          <div class="text-field">
            <span class="info-label">Text:</span>
            <textarea
              class="text-input"
              rows="2"
              placeholder={selectedDef?.procedural
                ? 'Sign text…'
                : 'Shown on hover…'}
              bind:value={textDraft}
              onchange={flushTextDraft}
              onblur={flushTextDraft}></textarea>
          </div>
        {/if}
        <button
          class="panel-btn flatten-btn"
          onclick={flattenTerrain}
          disabled={flattening || !heightManager}
        >
          {flattening ? 'Flattening…' : 'Flatten Terrain'}
        </button>
        <button class="panel-btn dup-btn" onclick={duplicateSelectedPlacement}>
          Duplicate (Ctrl+D)
        </button>
        <button class="panel-btn delete-btn" onclick={deleteSelectedPlacement}>
          Delete
        </button>
      </div>
    {:else}
      <div class="draw-hint">Click a placed object to select</div>
    {/if}
  {/if}
</div>

<style>
  .object-panel {
    background: rgba(0, 0, 0, 0.85);
    color: #e0e0e0;
    padding: 12px 16px;
    border-radius: 8px;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    border: 1px solid rgba(226, 185, 59, 0.3);
    box-shadow: 0 2px 12px rgba(0, 0, 0, 0.6);
    min-width: 240px;
    user-select: none;
  }

  .panel-title {
    color: #e2b93b;
    font-weight: bold;
    font-size: 13px;
    margin-bottom: 10px;
    letter-spacing: 1px;
  }

  .sub-tools {
    display: flex;
    gap: 2px;
    margin-bottom: 8px;
  }

  .sub-tool-btn {
    flex: 1;
    padding: 4px 8px;
    font-size: 10px;
    font-family: inherit;
    font-weight: bold;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.05);
    color: #888;
    cursor: pointer;
    letter-spacing: 0.5px;
  }

  .sub-tool-btn:hover {
    color: #ccc;
    background: rgba(255, 255, 255, 0.1);
  }

  .sub-tool-btn.active {
    background: rgba(226, 185, 59, 0.2);
    color: #e2b93b;
    border-color: rgba(226, 185, 59, 0.4);
  }

  .section-label {
    color: #888;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin-bottom: 4px;
    margin-top: 8px;
  }

  .object-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 150px;
    overflow-y: auto;
  }

  .object-item-btn {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 4px 8px;
    font-size: 11px;
    color: #999;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 3px;
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    width: 100%;
  }

  .object-item-btn:hover {
    color: #ccc;
    background: rgba(255, 255, 255, 0.1);
  }

  .object-item-btn.active {
    background: rgba(68, 204, 255, 0.15);
    border-color: rgba(68, 204, 255, 0.4);
    color: #44ccff;
  }

  .item-name {
    font-weight: bold;
  }

  .item-action {
    font-size: 9px;
    color: #666;
  }

  .rotation-display {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    background: rgba(68, 204, 255, 0.1);
    border: 1px solid rgba(68, 204, 255, 0.2);
    border-radius: 3px;
  }

  .rotation-value {
    color: #44ccff;
    font-weight: bold;
    font-size: 13px;
  }

  .rotation-hint {
    color: #666;
    font-size: 9px;
  }

  .selected-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .coord-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px;
    border-radius: 3px;
    font-size: 10px;
    background: rgba(68, 204, 255, 0.1);
    border: 1px solid rgba(68, 204, 255, 0.2);
  }

  .info-label {
    color: #888;
    width: 30px;
    flex-shrink: 0;
  }

  .info-value {
    color: #ccc;
    flex: 1;
  }

  .y-slider {
    flex: 1;
    min-width: 0;
    height: 14px;
    accent-color: #44ccff;
    margin: 0;
  }

  .y-value {
    color: #44ccff;
    font-variant-numeric: tabular-nums;
    width: 42px;
    text-align: right;
    flex-shrink: 0;
  }

  .text-field {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 3px 6px;
    border-radius: 3px;
    font-size: 10px;
    background: rgba(68, 204, 255, 0.1);
    border: 1px solid rgba(68, 204, 255, 0.2);
  }

  .text-field .info-label {
    padding-top: 3px;
  }

  .text-input {
    flex: 1;
    min-width: 0;
    resize: vertical;
    background: rgba(0, 0, 0, 0.4);
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 3px;
    color: #e0e0e0;
    font-family: inherit;
    font-size: 10px;
    padding: 3px 5px;
  }

  .text-input:focus {
    outline: none;
    border-color: rgba(68, 204, 255, 0.5);
  }

  .text-input::placeholder {
    color: #666;
  }

  .panel-btn {
    margin-top: 4px;
    width: 100%;
    padding: 5px;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    font-family: inherit;
    font-size: 11px;
    font-weight: bold;
  }

  .flatten-btn {
    background: rgba(68, 204, 255, 0.15);
    border-color: rgba(68, 204, 255, 0.4);
    color: #44ccff;
  }

  .flatten-btn:hover:not(:disabled) {
    background: rgba(68, 204, 255, 0.3);
  }

  .flatten-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .delete-btn {
    background: rgba(255, 60, 60, 0.2);
    border-color: rgba(255, 60, 60, 0.4);
    color: #ff6666;
  }

  .dup-btn {
    background: rgba(80, 160, 255, 0.2);
    border-color: rgba(80, 160, 255, 0.4);
    color: #7fb8ff;
  }

  .dup-btn:hover {
    background: rgba(80, 160, 255, 0.35);
  }

  .delete-btn:hover {
    background: rgba(255, 60, 60, 0.35);
  }

  .draw-hint {
    padding: 6px 8px;
    background: rgba(226, 185, 59, 0.1);
    border: 1px solid rgba(226, 185, 59, 0.2);
    border-radius: 4px;
    color: #ccc;
    font-size: 10px;
    text-align: center;
    margin-top: 8px;
  }
</style>

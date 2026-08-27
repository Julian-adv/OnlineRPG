<script lang="ts">
  import { onDestroy } from 'svelte'
  import {
    ROOM_TEMPLATES,
    STAIR_TEMPLATES,
    selectedRoomTemplate,
    placementRotation,
    placementFloorLevel,
    placementRoomType,
    wallTextureIndex,
    floorTextureIndex,
    roofTextureIndex,
    placementRoofType,
    placementPreview,
    housingEditorTool,
    selectedHouseId,
    selectedRoomIndex,
    deleteSelectedRoom,
    flattenSelectedRoomTerrain,
    reinstallSelectedHouse,
    WALL_VARIANT_OPTIONS,
    type RoomTemplate,
    type HousingEditorTool,
  } from '../../stores/housingEditorStore'
  import {
    clearDoorPair,
    getWallByDir,
    pairDoubleDoor,
  } from '../../managers/housing-passability'
  import type { WallDirection } from '../../utils/house-geometry'
  import type {
    HouseData,
    RoofRidgeDir,
    RoofType,
    RoomData,
    RoomType,
  } from '../../types/housing'
  import {
    HOUSING_TEXTURES,
    initHousingTextures,
    getTexturePreviewUrls,
  } from '../../utils/housing-textures'
  import { housingManager } from '../../managers/housingManager'
  import { MAX_FLOOR_LEVEL } from '../../utils/house-geo-utils'

  const FLOOR_LEVELS = Array.from({ length: MAX_FLOOR_LEVEL + 1 }, (_, i) => i)

  const toCSS = (c: number) => `#${c.toString(16).padStart(6, '0')}`
  const TEX_ENTRIES = HOUSING_TEXTURES.map((t, i) => ({
    idx: i,
    label: t.label,
    color: toCSS(t.fallbackColor),
    sortOrder: t.sortOrder ?? i,
    internal: t.internal ?? false,
  }))
    .filter((t) => !t.internal)
    .sort((a, b) => a.sortOrder - b.sortOrder)

  let texPreviews = $state<(string | null)[]>(HOUSING_TEXTURES.map(() => null))
  let openDropdown = $state<string | null>(null)

  initHousingTextures().then(() => {
    texPreviews = getTexturePreviewUrls()
  })

  let rotation = $state(0)
  let wallTex = $state(0)
  let floorTex = $state(0)
  let roofTex = $state(0)
  let roofType = $state<RoofType>('flat')
  let selected = $state<RoomTemplate | null>(null)
  let preview = $state<{ x: number; z: number } | null>(null)
  let tool = $state<HousingEditorTool>('place')
  let floorLvl = $state(0)
  let roomType = $state<RoomType>('normal')
  let editHouseId = $state<string | null>(null)
  let editRoomIdx = $state<number | null>(null)
  let editVersion = $state(0)
  let reinstalling = $state(false)

  // Derived: the room being edited (editVersion forces recompute after local edits)
  let editRoom = $derived.by(() => {
    void editVersion
    if (editHouseId == null || editRoomIdx == null) return null
    const house = housingManager.getHouseById(editHouseId)
    if (!house || editRoomIdx >= house.rooms.length) return null
    return house.rooms[editRoomIdx]
  })

  const unsubs = [
    placementRotation.subscribe((v) => (rotation = v)),
    wallTextureIndex.subscribe((v) => (wallTex = v)),
    floorTextureIndex.subscribe((v) => (floorTex = v)),
    roofTextureIndex.subscribe((v) => (roofTex = v)),
    placementRoofType.subscribe((v) => (roofType = v)),
    selectedRoomTemplate.subscribe((v) => (selected = v)),
    placementPreview.subscribe((v) => (preview = v)),
    housingEditorTool.subscribe((v) => (tool = v)),
    placementFloorLevel.subscribe((v) => (floorLvl = v)),
    placementRoomType.subscribe((v) => (roomType = v)),
    selectedHouseId.subscribe((v) => (editHouseId = v)),
    selectedRoomIndex.subscribe((v) => (editRoomIdx = v)),
  ]
  function onWindowClick(e: MouseEvent) {
    if (openDropdown && !(e.target as Element)?.closest('.tex-dropdown')) {
      openDropdown = null
    }
  }
  window.addEventListener('click', onWindowClick)

  onDestroy(() => {
    unsubs.forEach((u) => u())
    if (editSaveTimer) clearTimeout(editSaveTimer)
    window.removeEventListener('click', onWindowClick)
  })

  function setTool(t: HousingEditorTool) {
    housingEditorTool.set(t)
  }

  function selectTemplate(t: RoomTemplate) {
    housingEditorTool.set('place')
    selectedRoomTemplate.set(t)
  }

  function rotate() {
    placementRotation.set((rotation + 90) % 360)
  }

  const VARIANT_LABELS: Record<string, string> = {
    solid: '⬜',
    door: '🚪',
    'double-door': '⛩',
    window: '⊞',
  }

  // --- Edit mode functions ---

  let editSaveTimer: ReturnType<typeof setTimeout> | null = null
  let pendingHouse: HouseData | null = null

  function applyRoomEdit(
    mutateFn: (room: RoomData, house: HouseData, roomIndex: number) => void
  ) {
    if (editHouseId == null || editRoomIdx == null) return

    const house = housingManager.getHouseById(editHouseId)
    if (!house || editRoomIdx >= house.rooms.length) return

    const updatedHouse: HouseData = structuredClone(house)
    mutateFn(updatedHouse.rooms[editRoomIdx], updatedHouse, editRoomIdx)

    // Instant local update for visual feedback
    housingManager.updateLocalCache(updatedHouse)
    editVersion++

    // Debounce server save
    pendingHouse = updatedHouse
    if (editSaveTimer) clearTimeout(editSaveTimer)
    editSaveTimer = setTimeout(async () => {
      if (pendingHouse) {
        await housingManager.updateHouse(pendingHouse)
        pendingHouse = null
      }
    }, 300)
  }

  const WALL_DIRS: { label: string; dir: WallDirection }[] = [
    { label: 'N', dir: 'north' },
    { label: 'S', dir: 'south' },
    { label: 'E', dir: 'east' },
    { label: 'W', dir: 'west' },
  ]

  function cycleSegmentVariant(dir: WallDirection, segIdx: number) {
    applyRoomEdit((room, house, roomIndex) => {
      const segs = getWallByDir(room, dir)
      const seg = segs[segIdx]
      if (seg.variant === 'open') return
      const idx = WALL_VARIANT_OPTIONS.indexOf(seg.variant)
      let next = WALL_VARIANT_OPTIONS[(idx + 1) % WALL_VARIANT_OPTIONS.length]
      clearDoorPair(house.rooms, roomIndex, dir, segIdx)
      if (
        next === 'double-door' &&
        !pairDoubleDoor(house.rooms, roomIndex, dir, segIdx)
      )
        next = 'window'
      segs[segIdx] = { ...seg, variant: next }
    })
  }

  function onEditTextureChange(kind: 'wall' | 'floor' | 'roof', idx: number) {
    const store =
      kind === 'wall'
        ? wallTextureIndex
        : kind === 'floor'
          ? floorTextureIndex
          : roofTextureIndex
    store.set(idx)
    applyRoomEdit((room) => {
      if (kind === 'wall') {
        for (const { dir } of WALL_DIRS) {
          for (const seg of getWallByDir(room, dir)) {
            if (seg.variant !== 'open') seg.texture = idx
          }
        }
      } else if (kind === 'floor') {
        room.floorTexture = idx
      } else {
        room.roofTexture = idx
      }
    })
  }

  async function onReinstallHouse() {
    if (!reinstallSelectedHouse || reinstalling) return
    reinstalling = true
    try {
      await reinstallSelectedHouse()
    } finally {
      reinstalling = false
    }
  }
</script>

<div class="editor-mode-badge">
  HOUSING{#if preview}
    <span class="cell-info">
      ({preview.x.toFixed(0)}, {preview.z.toFixed(0)})
    </span>
  {/if}
</div>
<div class="editor-panel-container">
  <div class="panel">
    <div class="section-title">Tools</div>
    <div class="tool-row">
      <button
        class="tool-btn"
        class:active={tool === 'place'}
        onclick={() => setTool('place')}
      >
        Place
      </button>
      <button
        class="tool-btn tool-select"
        class:active={tool === 'select'}
        onclick={() => setTool('select')}
      >
        Select
      </button>
      {#if tool === 'select' && editHouseId != null && editRoomIdx != null}
        {#if editRoom && editRoom.floorLevel === 0 && editRoom.roomType !== 'stairwell'}
          <button
            class="tool-btn tool-flatten"
            onclick={() => flattenSelectedRoomTerrain?.()}>Flatten</button
          >
        {/if}
        <button
          class="tool-btn tool-reinstall"
          onclick={onReinstallHouse}
          disabled={reinstalling}
          >{reinstalling ? 'Reinstalling...' : 'Reinstall'}</button
        >
        <button
          class="tool-btn tool-delete"
          onclick={() => deleteSelectedRoom?.()}>Delete</button
        >
      {/if}
    </div>

    {#snippet texSwatch(texIdx: number)}
      {#if texPreviews[texIdx]}
        <img class="tex-swatch" src={texPreviews[texIdx]} alt="" />
      {:else}
        {@const entry = TEX_ENTRIES.find((e) => e.idx === texIdx)}
        <span class="tex-swatch" style="background: {entry?.color ?? '#888'}"
        ></span>
      {/if}
    {/snippet}

    {#snippet texturePicker(
      title: string,
      activeIdx: number,
      onChange: (idx: number) => void
    )}
      <div class="section-title">{title}</div>
      <div class="tex-dropdown">
        <button
          class="tex-dropdown-btn"
          onclick={() => {
            openDropdown = openDropdown === title ? null : title
          }}
        >
          {@render texSwatch(activeIdx)}
          <span>{TEX_ENTRIES.find((e) => e.idx === activeIdx)?.label}</span>
          <span class="tex-arrow">{openDropdown === title ? '▲' : '▼'}</span>
        </button>
        {#if openDropdown === title}
          <div class="tex-dropdown-list">
            {#each TEX_ENTRIES as entry (entry.idx)}
              <button
                class="tex-dropdown-item"
                class:active={activeIdx === entry.idx}
                onclick={() => {
                  onChange(entry.idx)
                  openDropdown = null
                }}
              >
                {@render texSwatch(entry.idx)}
                <span>{entry.label}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/snippet}

    <div class="section-title">Type</div>
    <div class="tool-row">
      <button
        class="tool-btn"
        class:active={roomType === 'normal'}
        disabled={tool !== 'place'}
        onclick={() => placementRoomType.set('normal')}>Room</button
      >
      <button
        class="tool-btn"
        class:active={roomType === 'stairwell'}
        disabled={tool !== 'place'}
        onclick={() => placementRoomType.set('stairwell')}>Stairs</button
      >
    </div>

    <div class="section-title">Floor</div>
    <div class="tool-row">
      {#each FLOOR_LEVELS as fl (fl)}
        <button
          class="tool-btn"
          class:active={floorLvl === fl}
          disabled={tool !== 'place'}
          onclick={() => placementFloorLevel.set(fl)}>{fl + 1}F</button
        >
      {/each}
    </div>

    <div class="section-title">
      {roomType === 'stairwell' ? 'Stairs' : 'Room'}
    </div>
    <div class="room-row">
      {#each roomType === 'stairwell' ? STAIR_TEMPLATES : ROOM_TEMPLATES as t (t.label)}
        <button
          class="room-btn"
          class:active={selected?.label === t.label && tool === 'place'}
          disabled={tool !== 'place'}
          onclick={() => selectTemplate(t)}
        >
          {t.sizeX}×{t.sizeZ}
        </button>
      {/each}
    </div>

    <div class="section-title">Rotate <span class="hint">(R)</span></div>
    <button class="rotate-btn" disabled={tool !== 'place'} onclick={rotate}
      >{rotation}°</button
    >

    {@render texturePicker('Wall', wallTex, (i) => {
      if (tool === 'select') onEditTextureChange('wall', i)
      else wallTextureIndex.set(i)
    })}
    {@render texturePicker('Floor', floorTex, (i) => {
      if (tool === 'select') onEditTextureChange('floor', i)
      else floorTextureIndex.set(i)
    })}
    {@render texturePicker('Roof', roofTex, (i) => {
      if (tool === 'select') onEditTextureChange('roof', i)
      else roofTextureIndex.set(i)
    })}

    <div class="section-title">Roof Shape</div>
    <div class="tool-row">
      {#each [['flat', 'Flat'], ['gabled', 'Gabled'], ['steep', 'Steep']] as [type, label] (type)}
        <button
          class="tool-btn"
          class:active={roofType === type}
          onclick={() => {
            placementRoofType.set(type as RoofType)
            if (tool === 'select')
              applyRoomEdit((room) => {
                room.roofType = type as RoofType
              })
          }}>{label}</button
        >
      {/each}
    </div>

    {#if tool === 'select' && editRoom && roofType !== 'flat'}
      <div class="section-title">Ridge Direction</div>
      <div class="tool-row">
        {#each [['auto', 'Auto'], ['x', 'X →'], ['z', 'Z ↓']] as [dir, label] (dir)}
          <button
            class="tool-btn"
            class:active={(editRoom.roofRidgeDir ?? 'auto') === dir}
            onclick={() => {
              applyRoomEdit((room) => {
                room.roofRidgeDir = dir as RoofRidgeDir
              })
            }}>{label}</button
          >
        {/each}
      </div>
    {/if}

    {#if tool === 'select' && editRoom && editRoomIdx != null}
      <div class="section-title">
        Editing Room {editRoomIdx + 1} ({editRoom.sizeX}×{editRoom.sizeZ})
      </div>

      {#each WALL_DIRS as dir (dir.dir)}
        {@const wall = getWallByDir(editRoom, dir.dir)}
        <div class="section-title">{dir.label} Wall ({wall.length} seg)</div>
        <div class="segment-row">
          {#each wall as seg, segIdx (segIdx)}
            <button
              class="variant-btn"
              disabled={seg.variant === 'open'}
              title="Seg {segIdx + 1}: {seg.variant}"
              onclick={() => cycleSegmentVariant(dir.dir, segIdx)}
              >{seg.variant === 'open'
                ? '−'
                : VARIANT_LABELS[seg.variant]}</button
            >
          {/each}
        </div>
      {/each}
    {:else if tool === 'select'}
      <div class="info-text">Click a house to select a room</div>
    {:else}
      <div class="info-text">Select a room size and click to place</div>
    {/if}
  </div>
</div>

<style>
  .editor-mode-badge {
    position: fixed;
    top: 10px;
    right: 10px;
    z-index: 1000;
    background: rgba(0, 0, 0, 0.8);
    color: #7bc67b;
    padding: 6px 12px;
    border-radius: 6px;
    font-family: 'Courier New', monospace;
    font-size: 13px;
    font-weight: bold;
    border: 1px solid rgba(123, 198, 123, 0.4);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.5);
    pointer-events: none;
    letter-spacing: 1px;
  }

  .cell-info {
    margin-left: 8px;
    color: #ccc;
    font-weight: normal;
    letter-spacing: 0;
  }

  .editor-panel-container {
    position: fixed;
    left: 16px;
    bottom: 16px;
    z-index: 1000;
    max-height: calc(100vh - 48px);
    overflow-y: auto;
  }

  .panel {
    background: rgba(0, 0, 0, 0.85);
    border-radius: 8px;
    padding: 10px 12px;
    border: 1px solid rgba(123, 198, 123, 0.3);
    font-family: 'Courier New', monospace;
    font-size: 12px;
    color: #ccc;
    min-width: 240px;
  }

  .section-title {
    color: #7bc67b;
    font-weight: bold;
    font-size: 11px;
    margin-top: 8px;
    margin-bottom: 4px;
    letter-spacing: 0.5px;
  }

  .section-title:first-child {
    margin-top: 0;
  }

  .hint {
    color: #666;
    font-weight: normal;
  }

  .tool-row {
    display: flex;
    gap: 3px;
  }

  .tool-btn {
    flex: 1;
    padding: 6px 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.05);
    color: #aaa;
    cursor: pointer;
    font-family: 'Courier New', monospace;
    font-size: 11px;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  .tool-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #ddd;
  }

  .tool-btn.active {
    background: rgba(123, 198, 123, 0.2);
    border-color: rgba(123, 198, 123, 0.5);
    color: #7bc67b;
  }

  .tool-select.active {
    background: rgba(68, 170, 255, 0.2);
    border-color: rgba(68, 170, 255, 0.5);
    color: #44aaff;
  }

  .tool-flatten {
    background: rgba(255, 200, 60, 0.15);
    border-color: rgba(255, 200, 60, 0.5);
    color: #ffc844;
  }

  .tool-flatten:hover {
    background: rgba(255, 200, 60, 0.3);
  }

  .tool-reinstall {
    background: rgba(90, 190, 220, 0.15);
    border-color: rgba(90, 190, 220, 0.5);
    color: #5fcbe6;
  }

  .tool-reinstall:hover:not(:disabled) {
    background: rgba(90, 190, 220, 0.3);
  }

  .tool-reinstall:disabled {
    cursor: wait;
    opacity: 0.6;
  }

  .tool-delete {
    background: rgba(255, 80, 80, 0.15);
    border-color: rgba(255, 80, 80, 0.5);
    color: #ff6666;
  }

  .tool-delete:hover {
    background: rgba(255, 80, 80, 0.3);
  }

  .info-text {
    color: #888;
    font-size: 11px;
    margin-top: 12px;
    text-align: center;
    padding: 8px;
  }

  .room-row {
    display: flex;
    gap: 3px;
  }

  .room-btn {
    flex: 1;
    padding: 5px 4px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.05);
    color: #aaa;
    cursor: pointer;
    font-family: 'Courier New', monospace;
    font-size: 11px;
    font-weight: bold;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  .room-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #ddd;
  }

  .room-btn.active {
    background: rgba(123, 198, 123, 0.2);
    border-color: rgba(123, 198, 123, 0.5);
    color: #7bc67b;
  }

  .rotate-btn {
    padding: 4px 12px;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.05);
    color: #aaa;
    cursor: pointer;
    font-family: 'Courier New', monospace;
    font-size: 12px;
  }

  .rotate-btn:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .variant-btn {
    width: 26px;
    height: 26px;
    padding: 0;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.05);
    color: #aaa;
    cursor: pointer;
    font-size: 12px;
    text-align: center;
    line-height: 26px;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  .variant-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.1);
    color: #ddd;
  }

  .variant-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .segment-row {
    display: flex;
    gap: 3px;
    flex-wrap: wrap;
  }

  .tex-dropdown {
    position: relative;
  }

  .tex-dropdown-btn {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.05);
    color: #ccc;
    cursor: pointer;
    font-family: 'Courier New', monospace;
    font-size: 11px;
    text-align: left;
  }

  .tex-dropdown-btn:hover {
    border-color: rgba(255, 255, 255, 0.3);
  }

  .tex-arrow {
    margin-left: auto;
    font-size: 8px;
    color: #888;
  }

  .tex-dropdown-list {
    position: absolute;
    left: 0;
    right: 0;
    top: 100%;
    z-index: 10;
    max-height: 200px;
    overflow-y: auto;
    background: rgba(20, 20, 20, 0.95);
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 4px;
    margin-top: 2px;
  }

  .tex-dropdown-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border: none;
    background: none;
    color: #ccc;
    cursor: pointer;
    font-family: 'Courier New', monospace;
    font-size: 11px;
    text-align: left;
  }

  .tex-dropdown-item:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .tex-dropdown-item.active {
    background: rgba(123, 198, 123, 0.15);
    color: #7bc67b;
  }

  .tex-swatch {
    display: inline-block;
    width: 18px;
    height: 18px;
    border-radius: 3px;
    flex-shrink: 0;
    object-fit: cover;
  }
</style>

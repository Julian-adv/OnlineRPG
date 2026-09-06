<script lang="ts">
  import {
    landscapingMode,
    landscapingPending,
    landscapingError,
    landscapingHint,
    hasLandscapingToolbox,
    selectLandscapingTool,
  } from '../stores/landscapingStore'
  import {
    fencePending,
    fenceError,
    fenceTarget,
    fenceCount,
    showFenceNoSpawnZones,
    stopFenceMode,
  } from '../stores/fenceStore'
  import type { LandscapingTool } from '../terrain/landscaping'
  import { isAdminUser } from '../stores/gameStore'
  import SplatBrushPanel from './map-editor/SplatBrushPanel.svelte'
  import { draggablePanel } from '../actions/draggablePanel'

  const tabs: LandscapingTool[] = ['Ground', 'Road', 'Fence']
</script>

{#if $landscapingMode}
  {@const status =
    $landscapingMode.tool === 'Fence'
      ? $fencePending
        ? 'Saving…'
        : ($fenceError ?? $fenceTarget?.reason)
      : $landscapingPending
        ? 'Saving…'
        : ($landscapingError ?? $landscapingHint)}
  <div class="landscaping-panel" use:draggablePanel={'landscaping'}>
    <div class="panel-header" data-drag-handle>
      <strong>Estate Landscaping</strong>
      <button
        class="close-btn"
        aria-label="Close landscaping"
        title="Close (Esc)"
        onclick={stopFenceMode}>×</button
      >
    </div>
    <div class="tabs" role="tablist" aria-label="Landscaping tools">
      {#each tabs as tab (tab)}
        <button
          role="tab"
          aria-selected={$landscapingMode.tool === tab}
          class:active={$landscapingMode.tool === tab}
          disabled={tab !== 'Fence' && !$hasLandscapingToolbox}
          title={tab !== 'Fence' && !$hasLandscapingToolbox
            ? "Carry a Landscaper's Toolbox to paint"
            : tab}
          onclick={() => selectLandscapingTool(tab)}>{tab}</button
        >
      {/each}
    </div>
    {#if $landscapingMode.tool === 'Fence'}
      <div class="fence-content">
        <strong>Wooden Fence · {$fenceCount} in bag</strong>
        {#if $isAdminUser}
          <label class="zone-toggle">
            <input type="checkbox" bind:checked={$showFenceNoSpawnZones} />
            Show no-spawn zones
          </label>
        {/if}
      </div>
    {:else}
      <SplatBrushPanel
        sizeLabel={$landscapingMode.tool === 'Road' ? 'Width' : 'Size'}
        title={$landscapingMode.tool === 'Ground'
          ? 'Ground Brush'
          : 'Road Tool'}
        hint={$landscapingMode.tool === 'Ground'
          ? '(drag to paint)'
          : '(click two points)'}
        availableLayers={$landscapingMode.palette}
      />
    {/if}
    {#if status}
      <div class="paint-status" role="status">{status}</div>
    {/if}
  </div>
{/if}

<style>
  .landscaping-panel {
    position: fixed;
    bottom: 100px;
    left: 16px;
    z-index: 40;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 120px);
    overflow: auto;
    border-radius: 8px;
    background: #211c16ed;
    color: #f3e8d2;
    pointer-events: auto;
  }
  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 4px 8px;
    font-family: 'Courier New', monospace;
    font-size: 13px;
  }
  button {
    cursor: pointer;
    color: inherit;
    background: #3a3024;
    border: 1px solid #766247;
    border-radius: 4px;
    padding: 5px 14px;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    font-weight: bold;
  }
  .close-btn {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    background: transparent;
    font-size: 16px;
    line-height: 1;
  }
  .close-btn:hover {
    background: #3a3024;
  }
  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .tabs {
    display: flex;
    gap: 2px;
    margin: 0 0 4px;
    padding: 2px;
    background: rgba(0, 0, 0, 0.7);
  }
  .tabs button {
    flex: 1;
    padding: 4px 10px;
    border: none;
    background: transparent;
    color: #888;
    letter-spacing: 0.5px;
    transition:
      background 150ms ease,
      color 150ms ease;
  }
  .tabs button:hover:not(:disabled) {
    color: #ccc;
  }
  .tabs button.active {
    background: rgba(226, 185, 59, 0.25);
    color: #e2b93b;
  }
  .landscaping-panel :global(.splat-brush-panel) {
    border: none;
    border-block: 1px solid rgba(226, 185, 59, 0.3);
    border-radius: 0;
    box-shadow: none;
  }
  .fence-content,
  .paint-status {
    display: grid;
    gap: 6px;
    padding: 12px 16px;
    font-family: 'Courier New', monospace;
    font-size: 12px;
  }
  .zone-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
  }
  .zone-toggle input {
    accent-color: #e2b93b;
  }
</style>

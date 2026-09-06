<script lang="ts">
  import {
    landscapingMode,
    landscapingPending,
    landscapingError,
    landscapingHint,
    landscapingRoadStart,
    hasLandscapingToolbox,
    selectLandscapingTool,
  } from '../stores/landscapingStore'
  import {
    fencePending,
    fenceError,
    fenceTarget,
    fenceCount,
    stopFenceMode,
  } from '../stores/fenceStore'
  import type { LandscapingTool } from '../terrain/landscaping'
  import SplatBrushPanel from './map-editor/SplatBrushPanel.svelte'

  const tabs: LandscapingTool[] = ['Ground', 'Road', 'Fence']
</script>

{#if $landscapingMode}
  <div class="landscaping-panel">
    <div class="panel-header">
      <strong>Estate Landscaping</strong>
      <button onclick={stopFenceMode}>Done</button>
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
        <span role="status"
          >{$fencePending
            ? 'Saving…'
            : ($fenceError ??
              $fenceTarget?.reason ??
              ($fenceTarget?.removing
                ? 'Click to recover this fence'
                : 'Point at a cell edge and click to place'))}</span
        >
        <small>Left-click to place or recover · Right-click to move</small>
      </div>
    {:else}
      <SplatBrushPanel
        title={$landscapingMode.tool === 'Ground'
          ? 'Ground Brush'
          : 'Road Tool'}
        hint={$landscapingMode.tool === 'Ground'
          ? '(drag to paint)'
          : '(click two points)'}
        availableLayers={$landscapingMode.palette}
      />
      <div class="paint-status" role="status">
        <span
          >{$landscapingPending
            ? 'Saving…'
            : ($landscapingError ??
              $landscapingHint ??
              ($landscapingMode.tool === 'Road'
                ? $landscapingRoadStart
                  ? 'Choose the end of your path'
                  : 'Choose the start of your path'
                : 'Drag to paint inside your estate'))}</span
        >
        <small>Painting is free · Learn more materials from Aldwin</small>
      </div>
    {/if}
    <small class="footer">Your estate only · Esc to finish</small>
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
    border: 1px solid #b49860;
    border-radius: 8px;
    background: #211c16ed;
    color: #f3e8d2;
    pointer-events: auto;
  }
  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 10px 14px;
  }
  button {
    cursor: pointer;
    color: inherit;
    background: #3a3024;
    border: 1px solid #766247;
    border-radius: 4px;
    padding: 5px 14px;
  }
  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .tabs {
    display: flex;
    gap: 5px;
    padding: 0 12px 8px;
  }
  .tabs .active {
    background: #745b32;
    border-color: #c6a56a;
  }
  .fence-content,
  .paint-status {
    display: grid;
    gap: 6px;
    padding: 12px 16px;
  }
  small {
    color: #c7baa4;
  }
  .footer {
    display: block;
    padding: 0 16px 12px;
  }
</style>

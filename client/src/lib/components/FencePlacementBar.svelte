<script lang="ts">
  import {
    fenceMode,
    fencePending,
    fenceError,
    fenceTarget,
    fenceCount,
    stopFenceMode,
  } from '../stores/fenceStore'
</script>

{#if $fenceMode}
  <div class="fence-bar" role="status">
    <strong>Wooden Fence · {$fenceCount} in bag</strong>
    <span
      >{$fencePending
        ? 'Saving…'
        : ($fenceError ??
          $fenceTarget?.reason ??
          ($fenceTarget?.removing
            ? 'Click to recover this fence'
            : 'Point at a cell edge and click to place'))}</span
    >
    <small
      >Left-click to place or recover · Right-click to move · Esc to finish</small
    >
    <button onclick={stopFenceMode}>Done</button>
  </div>
{/if}

<style>
  .fence-bar {
    position: fixed;
    bottom: 100px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 40;
    display: grid;
    gap: 6px;
    width: max-content;
    max-width: calc(100vw - 32px);
    padding: 14px 18px;
    border: 1px solid #b49860;
    border-radius: 8px;
    background: #211c16ed;
    color: #f3e8d2;
    text-align: center;
    pointer-events: auto;
  }
  small {
    color: #c7baa4;
  }
  button {
    justify-self: center;
    padding: 4px 18px;
    cursor: pointer;
  }
</style>

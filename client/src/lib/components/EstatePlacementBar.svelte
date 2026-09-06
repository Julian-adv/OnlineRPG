<script lang="ts">
  import {
    fenceMode,
    fencePending,
    fenceError,
    fenceTarget,
    fenceCount,
    stopFenceMode,
  } from '../stores/fenceStore'
  import {
    estateChestError,
    estateChestMode,
    estateChestPending,
    stopEstateChestMode,
  } from '../stores/estateStorageStore'
  import { playerVisualFloorLevel } from '../stores/housingStore'
  import { itemDisplayName } from '../data/itemDefs'
  import PlacementModeBar from './PlacementModeBar.svelte'

  const presentation = $derived.by(() => {
    if ($estateChestMode) {
      return {
        title: `${itemDisplayName($estateChestMode.item_def_id)} · ${$playerVisualFloorLevel + 1}F`,
        message: $estateChestPending
          ? 'Saving…'
          : ($estateChestError ??
            'Point inside your estate and click to place'),
        hint: 'Left-click to place · Right-click to move · Mouse wheel rotates · Esc to finish',
        stop: stopEstateChestMode,
      }
    }
    if ($fenceMode) {
      return {
        title: `${itemDisplayName('wooden_fence')} · ${$fenceCount} in bag`,
        message: $fencePending
          ? 'Saving…'
          : ($fenceError ??
            $fenceTarget?.reason ??
            ($fenceTarget?.removing
              ? 'Click to recover this fence'
              : 'Point at a cell edge and click to place')),
        hint: 'Left-click to place or recover · Right-click to move · Esc to finish',
        stop: stopFenceMode,
      }
    }
    return null
  })
</script>

{#if presentation}
  <PlacementModeBar
    title={presentation.title}
    message={presentation.message}
    hint={presentation.hint}
    onaction={presentation.stop}
  />
{/if}

<script lang="ts">
  import { statLabels, type ItemDefinition } from '../data/itemDefs'

  interface Props {
    def: ItemDefinition
    /** Enchantment level; prefixes the name (e.g. "+2 Iron Sword"). */
    enchant?: number
    side?: 'left' | 'right'
    anchor: DOMRect
  }

  // Mounted at document.body by the itemTooltip action; positions itself
  // next to the anchor rect, clamped to the viewport vertically.
  let { def, enchant = 0, side = 'right', anchor }: Props = $props()

  let height = $state(0)

  const top = $derived(
    Math.max(8, Math.min(anchor.top, window.innerHeight - height - 8))
  )
  const horizontal = $derived(
    side === 'left'
      ? `right: ${window.innerWidth - anchor.left + 8}px;`
      : `left: ${anchor.right + 8}px;`
  )
</script>

<div
  class="tooltip"
  style="top: {top}px; {horizontal}"
  bind:clientHeight={height}
>
  <div class="tooltip-name">{enchant > 0 ? `+${enchant} ` : ''}{def.name}</div>
  <div class="tooltip-desc">{def.description}</div>
  <div class="tooltip-stats">
    <span>Weight: {def.weight}</span>
    {#if def.equipSlot}
      <span>Slot: {def.equipSlot.replace(/_/g, ' ')}</span>
    {/if}
    {#if def.category === 'weapon' && def.dice}
      <span>Damage: {def.dice}{enchant > 0 ? `+${enchant}` : ''}</span>
    {:else if def.category === 'healing_potion' && def.dice}
      <span>Heals: {def.dice}</span>
    {/if}
    {#each statLabels(def, enchant) as label (label)}
      <span>{label}</span>
    {/each}
  </div>
</div>

<style>
  .tooltip {
    position: fixed;
    width: 160px;
    padding: 8px;
    background: rgba(6, 10, 14, 0.9);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 6px;
    pointer-events: none;
    z-index: 100;
    font-family: 'Courier New', monospace;
    color: #e6edf3;
  }

  .tooltip-name {
    font-size: 15px;
    font-weight: 700;
    color: #f0c040;
    margin-bottom: 4px;
  }

  .tooltip-desc {
    font-size: 13px;
    color: #9fb2c3;
    margin-bottom: 6px;
  }

  .tooltip-stats {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 13px;
    color: #c8d6e0;
  }
</style>

<script lang="ts">
  import { statLabels, type ItemDefinition } from '../data/itemDefs'

  interface Props {
    def: ItemDefinition
    /** Enchantment level; prefixes the name (e.g. "+2 Iron Sword"). */
    enchant?: number
    side?: 'left' | 'right'
    anchor: DOMRect
  }

  // Mounted at document.body by the itemTooltip action; sits beside the
  // anchor, clamped vertically and flipped sideways when it would overflow.
  let { def, enchant = 0, side = 'right', anchor }: Props = $props()

  const GAP = 8
  // .tooltip rendered width: 160px + 2*8px padding + 2*1px border
  const WIDTH = 178
  const vw = window.innerWidth
  const vh = window.innerHeight

  let height = $state(0)

  const top = $derived(Math.max(GAP, Math.min(anchor.top, vh - height - GAP)))
  const fitsLeft = $derived(anchor.left - GAP - WIDTH >= 0)
  const fitsRight = $derived(anchor.right + GAP + WIDTH <= vw)
  const onLeft = $derived(
    side === 'left' ? fitsLeft || !fitsRight : fitsLeft && !fitsRight
  )
  const horizontal = $derived(
    onLeft
      ? `right: ${vw - anchor.left + GAP}px;`
      : `left: ${anchor.right + GAP}px;`
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
    user-select: none;
    -webkit-user-select: none;
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

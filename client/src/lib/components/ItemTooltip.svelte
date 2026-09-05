<script lang="ts">
  import {
    compareStats,
    displayName,
    getItemDef,
    statLabels,
    type ItemDefinition,
    weaponTypeLabel,
  } from '../data/itemDefs'
  import { inventoryStore } from '../stores/inventoryStore'

  interface Props {
    def: ItemDefinition
    /** Enchantment level; prefixes the name (e.g. "+2 Iron Sword"). */
    enchant?: number
    side?: 'left' | 'right'
    anchor: DOMRect
    /** The equipped item this one would replace. */
    compare?: { def: ItemDefinition; enchant: number }
  }

  // Mounted at document.body by the itemTooltip action; sits beside the
  // anchor, clamped vertically and flipped sideways when it would overflow.
  let { def, enchant = 0, side = 'right', anchor, compare }: Props = $props()

  // A bow's own die is a token: what it hurts for depends on the round it
  // draws, so the comparison is made against that rather than the bow alone.
  const chosenAmmo = $derived(
    $inventoryStore.active_ammo
      ? getItemDef($inventoryStore.active_ammo)
      : undefined
  )
  /** The round this weapon would actually draw, or undefined when it draws
   *  none — either it is not a ranged weapon or the quiver is empty. */
  const drawnRound = $derived(
    def.ammoKind && chosenAmmo?.ammoKind === def.ammoKind
      ? chosenAmmo
      : undefined
  )
  const deltas = $derived(
    compare
      ? compareStats(def, enchant, compare.def, compare.enchant, chosenAmmo)
      : []
  )
  const fmt = (n: number) => String(+Math.abs(n).toFixed(1))

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
  <div class="tooltip-name">{displayName(def, enchant)}</div>
  <div class="tooltip-desc">{def.description}</div>
  <div class="tooltip-stats">
    <span>Weight: {def.weight}</span>
    {#if def.equipSlot}
      <span>Slot: {def.equipSlot.replace(/_/g, ' ')}</span>
    {/if}
    {#if def.weaponType}
      <span>Type: {weaponTypeLabel(def.weaponType)}</span>
    {/if}
    {#if def.category === 'weapon' && def.dice}
      <!-- A bow's own die is a token; the round it draws carries the rest, so
           quoting the bow alone reads as worthless. Both are shown, and where
           each comes from stays visible. -->
      <span>
        Damage: {def.dice}{drawnRound ? `+${drawnRound.dice}` : ''}{enchant > 0
          ? `+${enchant}`
          : ''}
      </span>
      {#if def.ammoKind && !drawnRound}
        <span class="tooltip-warn">No {def.ammoKind}s</span>
      {/if}
    {:else if def.ammoKind && def.dice}
      <!-- Ammunition adds its die to the weapon's rather than replacing it,
           so the sign says which. -->
      <span>Damage: +{def.dice}</span>
    {:else if def.category === 'healing_potion' && def.dice}
      <span>Heals: {def.dice}</span>
    {/if}
    {#each statLabels(def, enchant) as label (label)}
      <span>{label}</span>
    {/each}
  </div>
  {#if compare && deltas.length}
    <div class="tooltip-compare">
      <div class="compare-title">
        vs {displayName(compare.def, compare.enchant)}
      </div>
      {#each deltas as d (d.label)}
        <span class={d.better ? 'up' : 'down'}>
          {d.label}: {d.delta > 0 ? '▲' : '▼'}
          {fmt(d.delta)}
        </span>
      {/each}
    </div>
  {/if}
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

  .tooltip-compare {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 6px;
    padding-top: 6px;
    border-top: 1px solid rgba(255, 255, 255, 0.15);
    font-size: 13px;
  }

  .compare-title {
    color: #9fb2c3;
  }

  .up {
    color: #6cc8f0;
  }

  .down {
    color: #e06a6a;
  }

  .tooltip-stats {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 13px;
    color: #c8d6e0;
  }

  /* An empty quiver: the bow reads as its token die alone until it is fixed. */
  .tooltip-warn {
    color: #e08a8a;
  }
</style>

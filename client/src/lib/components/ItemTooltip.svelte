<script lang="ts">
  import { itemCoverageText, type ItemDefinition } from '../data/itemDefs'
  import { skillDisplayName } from '../stores/skillsStore'
  import { durability_condition } from '../wasm/onlinerpg_shared'

  interface Props {
    def: ItemDefinition
    /** Weapon enchantment level; prefixes the name (e.g. "+2 Iron Sword"). */
    enchant?: number
    /** Remaining per-instance condition for durable equipment. */
    durability?: number | null
    side?: 'left' | 'right'
    anchor: DOMRect
  }

  // Mounted at document.body by the itemTooltip action; positions itself
  // next to the anchor rect, clamped to the viewport vertically.
  let {
    def,
    enchant = 0,
    durability = null,
    side = 'right',
    anchor,
  }: Props = $props()

  let height = $state(0)

  const top = $derived(
    Math.max(8, Math.min(anchor.top, window.innerHeight - height - 8))
  )
  const horizontal = $derived(
    side === 'left'
      ? `right: ${window.innerWidth - anchor.left + 8}px;`
      : `left: ${anchor.right + 8}px;`
  )
  const physicalProtection = $derived(
    (['slash', 'pierce', 'blunt'] as const)
      .map((damageType) => ({
        damageType,
        amount:
          damageType === 'slash'
            ? (def.slashProtection ?? 0)
            : damageType === 'pierce'
              ? (def.pierceProtection ?? 0)
              : (def.bluntProtection ?? 0),
      }))
      .filter(({ amount }) => amount > 0)
  )
  const coverageText = $derived(itemCoverageText(def))
  const condition = $derived(
    def.maxDurability && durability !== null
      ? durability_condition(durability, def.maxDurability)
      : undefined
  )
  const conditionName = $derived(
    condition ? condition[0].toUpperCase() + condition.slice(1) : undefined
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
      {#if def.damageType}
        <span
          >Damage Type: {def.damageType[0].toUpperCase() +
            def.damageType.slice(1)}</span
        >
      {/if}
    {:else if def.category === 'healing_potion' && def.dice}
      <span>Heals: {def.dice}</span>
    {:else if def.category === 'bandage' && def.dice}
      <span>Treats: {def.dice}</span>
    {/if}
    {#if def.weaponSkill}
      <span>Weapon Skill: {skillDisplayName(def.weaponSkill)}</span>
    {/if}
    {#if def.defenseSkill}
      <span>Defense Skill: {skillDisplayName(def.defenseSkill)}</span>
    {/if}
    {#if def.useSkill}
      <span>Treatment Skill: {skillDisplayName(def.useSkill)}</span>
    {/if}
    {#if def.guard}
      <span>Guard: +{def.guard}</span>
    {/if}
    {#if def.maxDurability && durability !== null}
      <span class:broken={condition === 'broken'}
        >Condition: {durability}/{def.maxDurability}{conditionName
          ? ` (${conditionName})`
          : ''}</span
      >
    {/if}
    {#if def.repairFamily}
      <span
        >Repair Family: {def.repairFamily[0].toUpperCase() +
          def.repairFamily.slice(1)}</span
      >
    {/if}
    {#if def.repairAmount}
      <span>Repair Capacity: +{def.repairAmount} condition</span>
    {/if}
    {#if def.armorConstruction}
      <span
        >Construction: {def.armorConstruction[0].toUpperCase() +
          def.armorConstruction.slice(1)}</span
      >
    {/if}
    {#if physicalProtection.length > 0}
      <span
        >Protection: {physicalProtection
          .map(
            ({ damageType, amount }) =>
              `${damageType[0].toUpperCase() + damageType.slice(1)} ${amount}`
          )
          .join(', ')}</span
      >
    {/if}
    {#if def.equipmentKind}
      <span
        >Kind: {def.equipmentKind
          .split('_')
          .map((word) => word[0].toUpperCase() + word.slice(1))
          .join(' ')}</span
      >
    {/if}
    {#if def.equipmentLayer}
      <span
        >Layer: {def.equipmentLayer[0].toUpperCase() +
          def.equipmentLayer.slice(1)}</span
      >
    {/if}
    {#if def.garmentForm}
      <span
        >Form: {def.garmentForm[0].toUpperCase() +
          def.garmentForm.slice(1)}</span
      >
    {/if}
    {#if coverageText}
      <span>{coverageText}</span>
    {/if}
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

  .broken {
    color: #ff7373;
    font-weight: 700;
  }
</style>

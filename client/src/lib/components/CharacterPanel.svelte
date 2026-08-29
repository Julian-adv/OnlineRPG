<script lang="ts">
  import {
    inventoryStore,
    playerEffectiveStats,
  } from '../stores/inventoryStore'
  import type { EquipSlot } from '../stores/inventoryStore'
  import { getItemDef } from '../data/itemDefs'
  import { networkManager } from '../network/socket'
  import type {
    CharacterAttributes,
    CharacterClass,
    Gender,
  } from '../network/networkTypes'
  import { skill_xp_for_level, skill_level_cap } from '../wasm/onlinerpg_shared'
  import { levelProgress } from '../utils/xpProgress'
  import { equipBgCandidates, equipBgFilter } from '../utils/equipBackground'
  import { SvelteSet } from 'svelte/reactivity'
  import { skillsStore, SKILL_DISPLAY_NAMES } from '../stores/skillsStore'
  import type { SkillId, SkillProgress } from '../network/networkTypes'
  import {
    dragMeta,
    startDrag,
    isSlotCompatible,
    pointInRect,
    isOverAnyDialog,
    FALLBACK_ICON,
  } from '../stores/dragStore'
  import { itemTooltip } from '../actions/itemTooltip'
  import { draggablePanel } from '../actions/draggablePanel'
  import CharacterStatusPane from './CharacterStatusPane.svelte'
  import {
    characterPanelTab,
    type CharacterPanelTab,
  } from '../stores/debugStore'

  interface Props {
    visible: boolean
    name: string
    characterClass: CharacterClass
    level: number
    currentXp: number
    currentHp: number
    maxHp: number
    gender: Gender
    attributes: CharacterAttributes
    onClose: () => void
  }

  let {
    visible,
    name,
    characterClass,
    gender,
    level,
    currentXp,
    currentHp,
    maxHp,
    attributes,
    onClose,
  }: Props = $props()

  const equipBgList = $derived(equipBgCandidates(characterClass, gender))
  const failedEquipBgs = new SvelteSet<string>()
  const equipBg = $derived(
    equipBgList.find((path) => !failedEquipBgs.has(path)) ?? equipBgList.at(-1)
  )

  // Effective stats come from the server (EffectiveStatsUpdated) so they never
  // drift from its formula; fall back to base until the first update arrives.
  function withBonus(key: 'guard' | 'cha'): string {
    const base = attributes[key]
    const eff = $playerEffectiveStats?.[key] ?? base
    return eff > base ? `${eff} (+${eff - base})` : `${eff}`
  }

  const CLASS_LABELS: Record<CharacterClass, string> = {
    knight: 'Knight',
    barbarian: 'Barbarian',
    rogue: 'Rogue',
    caveman: 'Caveman',
    valkyrie: 'Valkyrie',
    ranger: 'Ranger',
    priest: 'Priest',
    bard: 'Bard',
    merchant: 'Merchant',
    guard: 'Guard',
  }

  const classLabel = $derived(
    characterClass === 'caveman' && gender === 'female'
      ? 'Cavewoman'
      : CLASS_LABELS[characterClass]
  )

  const TABS: CharacterPanelTab[] = ['stats', 'skills', 'status']

  const trainedSkills = $derived(
    (Object.entries($skillsStore.map) as [SkillId, SkillProgress][]).sort(
      ([a], [b]) => a.localeCompare(b)
    )
  )

  function skillProgressPct(progress: SkillProgress): number {
    if (progress.level >= skill_level_cap()) return 100
    const start = skill_xp_for_level(progress.level)
    const next = skill_xp_for_level(progress.level + 1)
    return Math.min(100, ((progress.xp - start) / (next - start)) * 100)
  }

  const EQUIP_SLOT_LABELS: Record<EquipSlot, string> = {
    head: 'Head',
    main_hand: 'Main Hand',
    off_hand: 'Off Hand',
    chest: 'Chest',
    ear: 'Ear',
    neck: 'Neck',
    belt: 'Belt',
    pants: 'Pants',
    boots: 'Boots',
    ring: 'Ring R',
    ring_left: 'Ring L',
    hands: 'Hands',
    back: 'Back',
    shirt: 'Shirt',
  }

  // null = wire slot without a panel cell yet (shirt until its items ship)
  const SLOT_POSITIONS: Record<
    EquipSlot,
    { top: number; left: number } | null
  > = {
    head: { top: 9, left: 50 },
    ear: { top: 20, left: 70 },
    neck: { top: 20, left: 30 },
    chest: { top: 30, left: 50 },
    hands: { top: 31, left: 10 },
    main_hand: { top: 45, left: 10 },
    off_hand: { top: 45, left: 90 },
    ring: { top: 59, left: 10 },
    ring_left: { top: 59, left: 90 },
    belt: { top: 45, left: 50 },
    pants: { top: 60, left: 50 },
    boots: { top: 88, left: 50 },
    back: { top: 31, left: 90 },
    shirt: null,
  }

  const VISIBLE_SLOTS = (
    Object.entries(SLOT_POSITIONS) as [
      EquipSlot,
      { top: number; left: number } | null,
    ][]
  ).flatMap(([slot, pos]) => (pos ? [{ slot, ...pos }] : []))

  const xpInfo = $derived(levelProgress(level, currentXp))

  function unequip(slot: EquipSlot) {
    networkManager.sendUnequipItem(slot)
  }

  function onEquipPointerDown(
    e: PointerEvent,
    slot: EquipSlot,
    item: { instance_id: number; item_def_id: string; enchant: number }
  ) {
    if (e.button !== 0) return
    e.preventDefault()
    const def = getItemDef(item.item_def_id)

    startDrag(
      e,
      {
        instanceId: item.instance_id,
        defId: item.item_def_id,
        enchant: item.enchant,
        equipSlot: def?.equipSlot ?? null,
        source: { type: 'equipped', slot },
        icon: def?.icon ?? FALLBACK_ICON,
      },
      (x, y) => {
        const invPanel = document.querySelector('[data-panel="inventory"]')
        if (invPanel && pointInRect(x, y, invPanel.getBoundingClientRect())) {
          networkManager.sendUnequipItem(slot)
          return
        }
        if (!isOverAnyDialog(x, y)) {
          networkManager.sendDropItem(item.instance_id)
        }
      }
    )
  }
</script>

{#if visible}
  <div
    class="character-panel"
    role="dialog"
    aria-label="Character"
    use:draggablePanel={'character'}
  >
    <div class="panel-header" data-drag-handle>
      <span class="panel-title">{name}</span>
      <span class="panel-class">{classLabel}</span>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <div class="panel-section">
      <div class="tab-row">
        {#each TABS as tab (tab)}
          <button
            class="tab"
            class:active={$characterPanelTab === tab}
            aria-pressed={$characterPanelTab === tab}
            onclick={() => characterPanelTab.set(tab)}>{tab}</button
          >
        {/each}
      </div>
      <!-- Stats pane stays laid out while hidden so the panel keeps its size -->
      <div class="tab-panes">
        <div
          class="pane-stats"
          class:pane-hidden={$characterPanelTab !== 'stats'}
        >
          <div class="stats-grid">
            <div class="stat-row">
              <span class="stat-label">Lv</span>
              <span class="stat-value level-value">{level}</span>
            </div>
            <div class="stat-row">
              <span class="stat-label">HP</span>
              <span class="stat-value hp-value">{currentHp}/{maxHp}</span>
            </div>
            <div class="stat-row">
              <span class="stat-label">Guard</span>
              <span class="stat-value guard-value">{withBonus('guard')}</span>
            </div>
            <div class="stat-row">
              <span class="stat-label">Str</span>
              <span class="stat-value">{attributes.str}</span>
            </div>
            <div class="stat-row">
              <span class="stat-label">Dex</span>
              <span class="stat-value">{attributes.dex}</span>
            </div>
            <div class="stat-row">
              <span class="stat-label">Con</span>
              <span class="stat-value">{attributes.con}</span>
            </div>
            <div class="stat-row">
              <span class="stat-label">Int</span>
              <span class="stat-value">{attributes.int}</span>
            </div>
            <div class="stat-row">
              <span class="stat-label">Wis</span>
              <span class="stat-value">{attributes.wis}</span>
            </div>
            <div class="stat-row">
              <span class="stat-label">Cha</span>
              <span class="stat-value">{withBonus('cha')}</span>
            </div>
          </div>
          <div class="exp-block">
            <div class="exp-header">
              <span class="stat-label exp-label">Exp</span>
              <span class="exp-text"
                >{xpInfo.gainedXp}/{xpInfo.neededXp} ({xpInfo.percent}%)</span
              >
            </div>
            <div
              class="exp-track"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={xpInfo.neededXp}
              aria-valuenow={xpInfo.gainedXp}
            >
              <span class="exp-fill" style={`width: ${xpInfo.progress * 100}%`}
              ></span>
            </div>
          </div>
          <div class="equip-section">
            <img
              class="equip-bg"
              src={equipBg}
              alt=""
              draggable="false"
              style:filter={equipBgFilter(equipBg)}
              onerror={() => {
                if (equipBg) failedEquipBgs.add(equipBg)
              }}
            />
            {#each VISIBLE_SLOTS as { slot, top, left } (slot)}
              {@const item = $inventoryStore.equipped[slot]}
              {@const def = item ? getItemDef(item.item_def_id) : null}
              {@const isDropTarget =
                $dragMeta && isSlotCompatible($dragMeta.equipSlot, slot)}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="equip-slot"
                class:drop-target={isDropTarget}
                style="top:{top}%;left:{left}%"
                title={item ? undefined : EQUIP_SLOT_LABELS[slot]}
                data-equip-slot={slot}
                use:itemTooltip={item && def
                  ? { def, item, side: left > 50 ? 'left' : 'right' }
                  : null}
                ondblclick={() => {
                  if (item) unequip(slot)
                }}
                onpointerdown={(e: PointerEvent) => {
                  if (item) onEquipPointerDown(e, slot, item)
                }}
              >
                {#if def}
                  <img
                    class="equip-icon"
                    src="/items/{def.icon}"
                    alt={def.name}
                    draggable="false"
                  />
                {/if}
                {#if item && item.enchant > 0}
                  <span class="item-enchant">+{item.enchant}</span>
                {/if}
              </div>
            {/each}
          </div>
        </div>
        {#if $characterPanelTab === 'skills'}
          <div class="pane-skills">
            {#if trainedSkills.length > 0}
              <div class="skills-list">
                {#each trainedSkills as [skillId, progress] (skillId)}
                  <div class="skill-row">
                    <span class="stat-label"
                      >{SKILL_DISPLAY_NAMES[skillId] ?? skillId}</span
                    >
                    <span class="stat-value">Lv {progress.level}</span>
                    <div
                      class="skill-track"
                      role="progressbar"
                      aria-valuemin={0}
                      aria-valuemax={100}
                      aria-valuenow={Math.round(skillProgressPct(progress))}
                    >
                      <span
                        class="skill-fill"
                        style={`width: ${skillProgressPct(progress)}%`}
                      ></span>
                    </div>
                  </div>
                {/each}
              </div>
            {:else}
              <div class="skills-empty">No skills trained yet</div>
            {/if}
          </div>
        {/if}
        {#if $characterPanelTab === 'status'}
          <div class="pane-status">
            <CharacterStatusPane />
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .character-panel {
    --character-panel-width: 364px;
    --equip-section-height: 540px;
    --equip-slot-size: 64px;
    --equip-icon-size: 56px;
    --section-gap: 8px;
    position: fixed;
    left: 16px;
    top: 45%;
    transform: translateY(-50%);
    z-index: 40;
    width: var(--character-panel-width);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    backdrop-filter: blur(4px);
    padding: 10px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 10px;
    background: rgba(6, 10, 14, 0.88);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    pointer-events: auto;
  }

  .panel-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: 8px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.15);
    margin-bottom: 8px;
  }

  .panel-title {
    font-size: 14px;
    font-weight: 700;
    color: #f0c040;
  }

  .close-btn {
    background: none;
    border: none;
    color: #9fb2c3;
    font-size: 18px;
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #fff;
  }

  .panel-class {
    font-size: 11px;
    color: #9fb2c3;
  }

  .panel-section {
    margin-bottom: var(--section-gap);
  }

  .tab-panes {
    position: relative;
  }

  .pane-stats {
    display: flex;
    flex-direction: column;
    gap: var(--section-gap);
  }

  .pane-hidden {
    visibility: hidden;
  }

  .pane-skills,
  .pane-status {
    position: absolute;
    inset: 0;
    overflow-y: auto;
  }

  .tab-row {
    display: flex;
    gap: 6px;
    margin-bottom: 4px;
  }

  .tab {
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    padding: 3px 8px;
    font-family: inherit;
    font-size: 11px;
    color: #64798c;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    cursor: pointer;
  }

  .tab:hover {
    color: #cfe3ff;
  }

  .tab.active {
    color: #9fc5ff;
    border-color: rgba(159, 197, 255, 0.45);
    background: rgba(159, 197, 255, 0.08);
  }

  .stats-grid {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 2px;
  }

  .stat-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 4px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.04);
  }

  .stat-label {
    font-size: 10px;
    color: #9fb2c3;
    min-width: 34px;
  }

  .stat-value {
    font-size: 13px;
    font-weight: 700;
    color: #f5f9fc;
  }

  .level-value {
    color: #f0c040;
  }

  .hp-value {
    color: #6ee7b7;
  }

  .guard-value {
    color: #a78bfa;
  }

  .exp-block {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .exp-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
  }

  .exp-label {
    color: #9fc5ff;
  }

  .exp-text {
    font-size: 11px;
    color: #d5e5f6;
  }

  .exp-track {
    position: relative;
    height: 7px;
    border-radius: 999px;
    overflow: hidden;
    background: rgba(64, 98, 135, 0.45);
    border: 1px solid rgba(166, 200, 238, 0.25);
  }

  .exp-fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: linear-gradient(90deg, #58a6ff 0%, #7fd0ff 100%);
    box-shadow: 0 0 10px rgba(88, 166, 255, 0.4);
  }

  .skills-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .skill-row {
    display: grid;
    grid-template-columns: auto auto 1fr;
    align-items: center;
    gap: 8px;
  }

  /* Same track treatment as the character exp bar, green for skill growth. */
  .skill-track {
    position: relative;
    height: 7px;
    border-radius: 999px;
    overflow: hidden;
    background: rgba(64, 98, 135, 0.45);
    border: 1px solid rgba(166, 200, 238, 0.25);
  }

  .skill-fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: linear-gradient(90deg, #4fd58a 0%, #8be8b6 100%);
    box-shadow: 0 0 10px rgba(79, 213, 138, 0.4);
  }

  .skills-empty {
    padding: 4px 0;
    color: #9fb2c3;
    font-size: 11px;
  }

  .equip-section {
    position: relative;
    border-radius: 6px;
    min-height: var(--equip-section-height);
  }

  .equip-bg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    object-position: center bottom;
    opacity: 0.4;
    pointer-events: none;
  }

  .equip-slot {
    position: absolute;
    width: var(--equip-slot-size);
    height: var(--equip-slot-size);
    transform: translate(-50%, -50%);
    border: 1px solid rgba(255, 255, 255, 0.3);
    border-radius: 6px;
    background: rgba(148, 156, 164, 0.34);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  .equip-slot:hover {
    border-color: rgba(240, 192, 64, 0.6);
    background: rgba(166, 158, 126, 0.42);
    z-index: 10;
  }

  .equip-slot.drop-target {
    border-color: rgba(88, 255, 88, 0.8);
    background: rgba(88, 160, 88, 0.4);
    box-shadow: 0 0 8px rgba(88, 255, 88, 0.4);
  }

  .equip-icon {
    width: var(--equip-icon-size);
    height: var(--equip-icon-size);
    image-rendering: pixelated;
    pointer-events: none;
  }

  .item-enchant {
    position: absolute;
    top: 2px;
    left: 4px;
    font-size: 11px;
    font-weight: 700;
    color: #7ec8ff;
    text-shadow: 0 0 3px rgba(0, 0, 0, 0.8);
    pointer-events: none;
  }

  @media (max-width: 600px), (pointer: coarse) {
    .character-panel {
      --character-panel-width: min(
        292px,
        calc(
          100vw - 16px - env(safe-area-inset-left) - env(safe-area-inset-right)
        )
      );
      --equip-section-height: min(
        330px,
        calc(
          100dvh - 228px - env(safe-area-inset-top) -
            env(safe-area-inset-bottom)
        )
      );
      --equip-slot-size: 44px;
      --equip-icon-size: 38px;
      --section-gap: 6px;
      left: calc(8px + env(safe-area-inset-left));
      top: calc(8px + env(safe-area-inset-top));
      transform: none;
      max-height: calc(
        100dvh - 16px - env(safe-area-inset-top) - env(safe-area-inset-bottom)
      );
      padding: 8px;
      border-radius: 8px;
      overflow-y: auto;
      overscroll-behavior: contain;
      -webkit-overflow-scrolling: touch;
    }

    .panel-header {
      padding-bottom: 6px;
      margin-bottom: 6px;
      gap: 8px;
    }

    .panel-title {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-size: 13px;
    }

    .panel-class {
      font-size: 10px;
      white-space: nowrap;
    }

    .close-btn {
      min-width: 32px;
      min-height: 32px;
      font-size: 22px;
    }

    .tab-row {
      margin-bottom: 3px;
    }

    .tab {
      font-size: 10px;
      min-height: 24px;
    }

    .stat-row {
      gap: 4px;
      padding: 2px 3px;
    }

    .stat-label {
      min-width: 28px;
      font-size: 9px;
    }

    .stat-value {
      font-size: 11px;
    }

    .exp-header {
      gap: 6px;
    }

    .exp-text {
      font-size: 10px;
    }

    .exp-track {
      height: 6px;
    }
  }

  @media (max-width: 340px) {
    .character-panel {
      --equip-section-height: min(
        300px,
        calc(
          100dvh - 220px - env(safe-area-inset-top) -
            env(safe-area-inset-bottom)
        )
      );
      --equip-slot-size: 40px;
      --equip-icon-size: 34px;
    }
  }
</style>

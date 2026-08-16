<script lang="ts">
  import { hungerState, grilling } from '../stores/hungerStore'
  import { visibleDebuffs } from '../stores/debuffStore'
  import {
    characterPanelTab,
    characterPanelVisible,
  } from '../stores/debugStore'
  import {
    HUNGER_BAND_INFO,
    formatHungerModifier,
    hungerModifiers,
  } from '../data/hungerPresentation'

  const hungry = $derived($hungerState?.band === 'Hungry')
  const weak = $derived($hungerState?.band === 'Weak')
  const satiationPct = $derived(($hungerState?.satiation ?? 0) / 10)
  const modifiers = $derived($hungerState ? hungerModifiers($hungerState) : [])
  const accessibleSummary = $derived.by(() => {
    const h = $hungerState
    if (!h) return ''
    const lines = [`Open character status. Satiation ${h.satiation}/1000`]
    if (h.band === 'Hungry')
      lines.push('Sprinting disabled, natural healing takes twice as long')
    if (h.band === 'Weak')
      lines.push('Slowed, weaker carry, no natural healing — eat!')
    for (const d of $visibleDebuffs) lines.push(`${d.label}: ${d.note}`)
    return lines.join('\n')
  })

  function openStatusTab() {
    characterPanelTab.set('status')
    characterPanelVisible.set(true)
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter' && event.key !== ' ') return
    event.preventDefault()
    openStatusTab()
  }
</script>

{#if $hungerState}
  <div
    class="hunger"
    class:hungry
    class:weak
    class:debuffed={$visibleDebuffs.length > 0}
    aria-label={accessibleSummary}
    aria-describedby="hunger-tooltip"
    role="button"
    tabindex="0"
    onclick={openStatusTab}
    onkeydown={handleKeydown}
  >
    <span class="badge primary-badge">
      {HUNGER_BAND_INFO[$hungerState.band].icon}
      {HUNGER_BAND_INFO[$hungerState.band].label}
    </span>
    {#each $visibleDebuffs as debuff (debuff.id)}
      <span class="badge debuff">{debuff.icon} {debuff.remaining}</span>
    {/each}
    {#if $grilling}
      <span class="badge grilling">🐟 Grilling…</span>
    {/if}

    <div id="hunger-tooltip" class="hunger-tooltip" role="tooltip">
      <div class="tooltip-header">
        <span class="tooltip-icon"
          >{HUNGER_BAND_INFO[$hungerState.band].icon}</span
        >
        <div>
          <div class="tooltip-title">
            {HUNGER_BAND_INFO[$hungerState.band].label}
          </div>
          <div class="tooltip-description">
            {HUNGER_BAND_INFO[$hungerState.band].description}
          </div>
        </div>
      </div>

      <div class="satiation-heading">
        <span>Satiation</span>
        <strong>{$hungerState.satiation}<small>/1000</small></strong>
      </div>
      <div class="satiation-track">
        <div class="satiation-fill" style:width={`${satiationPct}%`}></div>
      </div>

      <div class="modifier-grid">
        {#each modifiers as modifier (modifier.label)}
          <div class="modifier">
            <span>{modifier.label}</span>
            <strong
              class:positive={modifier.mult > 1}
              class:negative={modifier.mult < 1}
              >{formatHungerModifier(modifier.mult)}</strong
            >
          </div>
        {/each}
      </div>

      {#if HUNGER_BAND_INFO[$hungerState.band].note}
        <div class="tooltip-note">
          {HUNGER_BAND_INFO[$hungerState.band].note}
        </div>
      {/if}
      {#each $visibleDebuffs as debuff (debuff.id)}
        <div class="debuff-warning">
          <span>{debuff.icon}</span>
          <div>
            <strong>{debuff.label}</strong>
            <small
              >{debuff.note ? `${debuff.note} · ` : ''}{debuff.remaining} remaining</small
            >
          </div>
        </div>
      {/each}
      {#if $grilling}
        <div class="grilling-note">🐟 Grilling in progress…</div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .hunger {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    justify-content: flex-start;
    cursor: pointer;
    outline: none;
  }

  .hunger:focus-visible {
    border-radius: 12px;
    box-shadow: 0 0 0 2px rgba(159, 197, 255, 0.7);
  }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border-radius: 11px;
    font-size: 12px;
    line-height: 1.4;
    color: #d8d2c4;
    background: rgba(20, 16, 10, 0.72);
    border: 1px solid rgba(255, 255, 255, 0.12);
    white-space: nowrap;
    user-select: none;
  }

  .hunger.weak .primary-badge {
    color: #f0b8a8;
    border-color: rgba(240, 120, 90, 0.45);
  }

  .badge.debuff,
  .hunger.debuffed .primary-badge {
    color: #f0b8a8;
    border-color: rgba(240, 120, 90, 0.45);
  }

  .badge.grilling {
    color: #f4d08a;
  }

  .hunger-tooltip {
    position: absolute;
    top: 0;
    left: calc(100% + 10px);
    z-index: 1200;
    box-sizing: border-box;
    width: 270px;
    padding: 13px;
    border: 1px solid rgba(216, 210, 196, 0.2);
    border-radius: 10px;
    background:
      radial-gradient(
        circle at top left,
        rgba(212, 167, 82, 0.12),
        transparent 45%
      ),
      rgba(12, 13, 14, 0.96);
    box-shadow:
      0 10px 28px rgba(0, 0, 0, 0.55),
      inset 0 1px rgba(255, 255, 255, 0.04);
    color: #d8d2c4;
    font-family: system-ui, sans-serif;
    pointer-events: none;
    opacity: 0;
    visibility: hidden;
    transform: translateX(-4px);
    transform-origin: left top;
    transition:
      opacity 120ms ease,
      transform 120ms ease,
      visibility 120ms;
  }

  .hunger-tooltip::before {
    position: absolute;
    top: 12px;
    left: -5px;
    width: 9px;
    height: 9px;
    border-left: 1px solid rgba(216, 210, 196, 0.2);
    border-bottom: 1px solid rgba(216, 210, 196, 0.2);
    background: #111213;
    content: '';
    transform: rotate(45deg);
  }

  .hunger:hover .hunger-tooltip {
    opacity: 1;
    visibility: visible;
    transform: translateX(0);
  }

  .hunger.weak .hunger-tooltip,
  .hunger.debuffed .hunger-tooltip {
    border-color: rgba(240, 120, 90, 0.38);
  }

  .tooltip-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .tooltip-icon {
    display: grid;
    flex: 0 0 34px;
    width: 34px;
    height: 34px;
    place-items: center;
    border: 1px solid rgba(255, 255, 255, 0.09);
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.04);
    font-size: 18px;
  }

  .tooltip-title {
    color: #f1eadb;
    font-size: 14px;
    font-weight: 700;
    line-height: 1.2;
  }

  .tooltip-description {
    margin-top: 2px;
    color: #99978f;
    font-size: 11px;
    line-height: 1.35;
  }

  .satiation-heading {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-top: 13px;
    color: #aaa79f;
    font-size: 11px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .satiation-heading strong {
    color: #e8dfcf;
    font-size: 12px;
    letter-spacing: 0;
  }

  .satiation-heading small {
    color: #77756f;
    font-size: 9px;
    font-weight: 500;
  }

  .satiation-track {
    height: 5px;
    margin-top: 5px;
    overflow: hidden;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.08);
  }

  .satiation-fill {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, #9d7437, #d4ad63);
    box-shadow: 0 0 8px rgba(212, 173, 99, 0.3);
  }

  .weak .satiation-fill,
  .debuffed .satiation-fill {
    background: linear-gradient(90deg, #914f43, #d77b67);
    box-shadow: 0 0 8px rgba(215, 123, 103, 0.28);
  }

  .modifier-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
    margin-top: 12px;
  }

  .modifier {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 7px 8px;
    border: 1px solid rgba(255, 255, 255, 0.07);
    border-radius: 7px;
    background: rgba(255, 255, 255, 0.025);
  }

  .modifier span {
    color: #7f817e;
    font-size: 9px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .modifier strong {
    color: #b7b4ad;
    font-size: 13px;
  }

  .modifier strong.positive {
    color: #a9d990;
  }

  .modifier strong.negative {
    color: #e28e7b;
  }

  .tooltip-note,
  .grilling-note {
    margin-top: 9px;
    padding: 7px 9px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.035);
    color: #aaa79f;
    font-size: 11px;
  }

  .grilling-note {
    color: #e5bf78;
  }

  .debuff-warning {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    padding: 8px 9px;
    border: 1px solid rgba(240, 120, 90, 0.25);
    border-radius: 7px;
    background: rgba(140, 60, 45, 0.12);
  }

  .debuff-warning > span {
    font-size: 16px;
  }

  .debuff-warning div {
    display: flex;
    flex-direction: column;
  }

  .debuff-warning strong {
    color: #f0b8a8;
    font-size: 11px;
  }

  .debuff-warning small {
    margin-top: 1px;
    color: #a98a80;
    font-size: 10px;
  }

  @media (max-width: 600px) {
    .hunger-tooltip {
      top: calc(100% + 8px);
      left: 0;
      width: min(270px, calc(100vw - 18px));
      transform: translateY(-4px);
    }

    .hunger-tooltip::before {
      top: -5px;
      left: 14px;
      border-top: 1px solid rgba(216, 210, 196, 0.2);
      border-left: 1px solid rgba(216, 210, 196, 0.2);
      border-bottom: 0;
    }

    .hunger:hover .hunger-tooltip {
      transform: translateY(0);
    }
  }
</style>

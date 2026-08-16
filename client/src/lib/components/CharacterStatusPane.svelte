<script lang="ts">
  import { hungerState, grilling } from '../stores/hungerStore'
  import { visibleDebuffs } from '../stores/debuffStore'
  import {
    HUNGER_BAND_INFO,
    formatHungerModifier,
    hungerModifiers,
  } from '../data/hungerPresentation'

  const satiationPct = $derived(($hungerState?.satiation ?? 0) / 10)
  const modifiers = $derived($hungerState ? hungerModifiers($hungerState) : [])
</script>

{#if $hungerState}
  <div
    class="status-content"
    class:hungry={$hungerState.band === 'Hungry'}
    class:weak={$hungerState.band === 'Weak'}
    class:debuffed={$visibleDebuffs.length > 0}
  >
    <section class="status-card" aria-label="Hunger status">
      <div class="status-header">
        <span class="status-icon"
          >{HUNGER_BAND_INFO[$hungerState.band].icon}</span
        >
        <div>
          <div class="status-title">
            {HUNGER_BAND_INFO[$hungerState.band].label}
          </div>
          <div class="status-description">
            {HUNGER_BAND_INFO[$hungerState.band].description}
          </div>
        </div>
      </div>

      <div class="satiation-heading">
        <span>Satiation</span>
        <strong>{$hungerState.satiation}<small>/1000</small></strong>
      </div>
      <div
        class="satiation-track"
        role="progressbar"
        aria-label="Satiation"
        aria-valuemin={0}
        aria-valuemax={1000}
        aria-valuenow={$hungerState.satiation}
      >
        <span class="satiation-fill" style:width={`${satiationPct}%`}></span>
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
        <div class="status-note">
          {HUNGER_BAND_INFO[$hungerState.band].note}
        </div>
      {/if}
    </section>

    {#each $visibleDebuffs as debuff (debuff.id)}
      <section class="effect-card debuff-card" aria-label={debuff.label}>
        <span class="effect-icon">{debuff.icon}</span>
        <div>
          <strong>{debuff.label}</strong>
          <small
            >{debuff.note ? `${debuff.note} · ` : ''}{debuff.remaining} remaining</small
          >
        </div>
      </section>
    {/each}

    {#if $grilling}
      <section class="effect-card grilling-card" aria-label="Grilling">
        <span class="effect-icon">🐟</span>
        <div>
          <strong>Grilling</strong>
          <small>Grilling in progress…</small>
        </div>
      </section>
    {/if}
  </div>
{:else}
  <div class="status-empty">Status unavailable</div>
{/if}

<style>
  .status-content {
    display: flex;
    flex-direction: column;
    gap: 10px;
    color: #d8d2c4;
    font-family: system-ui, sans-serif;
  }

  .status-card {
    padding: 13px;
    border: 1px solid rgba(216, 210, 196, 0.2);
    border-radius: 10px;
    background:
      radial-gradient(
        circle at top left,
        rgba(212, 167, 82, 0.12),
        transparent 45%
      ),
      rgba(12, 13, 14, 0.72);
  }

  .weak .status-card,
  .debuffed .status-card {
    border-color: rgba(240, 120, 90, 0.38);
  }

  .status-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .status-icon {
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

  .status-title {
    color: #f1eadb;
    font-size: 14px;
    font-weight: 700;
    line-height: 1.2;
  }

  .status-description {
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
    display: block;
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

  .status-note {
    margin-top: 9px;
    padding: 7px 9px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.035);
    color: #aaa79f;
    font-size: 11px;
  }

  .effect-card {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 9px 10px;
    border-radius: 7px;
  }

  .effect-card div {
    display: flex;
    flex-direction: column;
  }

  .effect-card strong {
    font-size: 11px;
  }

  .effect-card small {
    margin-top: 1px;
    font-size: 10px;
  }

  .effect-icon {
    font-size: 16px;
  }

  .debuff-card {
    border: 1px solid rgba(240, 120, 90, 0.25);
    background: rgba(140, 60, 45, 0.12);
  }

  .debuff-card strong {
    color: #f0b8a8;
  }

  .debuff-card small {
    color: #a98a80;
  }

  .grilling-card {
    border: 1px solid rgba(229, 191, 120, 0.2);
    background: rgba(229, 191, 120, 0.07);
  }

  .grilling-card strong {
    color: #e5bf78;
  }

  .grilling-card small {
    color: #a9936d;
  }

  .status-empty {
    padding: 4px 0;
    color: #9fb2c3;
    font-size: 11px;
  }
</style>

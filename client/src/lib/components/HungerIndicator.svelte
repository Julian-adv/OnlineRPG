<script lang="ts">
  import { hungerState, grilling, type HungerBand } from '../stores/hungerStore'

  const BAND_LABEL: Record<HungerBand, string> = {
    WellFed: 'Well Fed',
    Hungry: 'Hungry',
    Weak: 'Weak',
    Stuffed: 'Stuffed',
  }
  const BAND_ICON: Record<HungerBand, string> = {
    WellFed: '🍖',
    Hungry: '🍽️',
    Weak: '🦴',
    Stuffed: '🫃',
  }

  let poisonRemainingMin = $state(0)
  $effect(() => {
    const until = $hungerState?.poisonedUntil
    if (until == null) {
      poisonRemainingMin = 0
      return
    }
    const tick = () => {
      poisonRemainingMin = Math.max(0, Math.ceil((until - Date.now()) / 60_000))
    }
    tick()
    const timer = setInterval(tick, 5_000)
    return () => clearInterval(timer)
  })

  const buffed = $derived($hungerState?.band === 'WellFed')
  const weak = $derived($hungerState?.band === 'Weak')
  const pct = (mult: number) =>
    `${mult > 1 ? '+' : ''}${Math.round((mult - 1) * 100)}%`
  const tooltip = $derived.by(() => {
    const h = $hungerState
    if (!h) return ''
    const lines = [`Satiation ${h.satiation}/1000`]
    if (h.band === 'WellFed')
      lines.push(
        `${pct(h.moveMult)} move & attack speed, ${pct(h.carryMult)} carry weight`
      )
    if (h.band === 'Weak')
      lines.push('Slowed, weaker carry, no natural healing — eat!')
    if (h.band === 'Stuffed')
      lines.push('Too full — the vigor returns as it settles')
    if (h.poisonedUntil != null) lines.push('Food poisoning: heavy penalties')
    return lines.join('\n')
  })
</script>

{#if $hungerState}
  <div class="hunger" title={tooltip}>
    <span
      class="badge"
      class:buffed
      class:weak
      class:poisoned={$hungerState.poisonedUntil != null}
    >
      {BAND_ICON[$hungerState.band]}
      {BAND_LABEL[$hungerState.band]}
    </span>
    {#if $hungerState.poisonedUntil != null && poisonRemainingMin > 0}
      <span class="badge poisoned">☠️ {poisonRemainingMin}m</span>
    {/if}
    {#if $grilling}
      <span class="badge grilling">🐟 Grilling…</span>
    {/if}
  </div>
{/if}

<style>
  .hunger {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
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

  .badge.buffed {
    color: #b8e6a3;
    border-color: rgba(140, 220, 110, 0.35);
  }

  .badge.weak {
    color: #f0b8a8;
    border-color: rgba(240, 120, 90, 0.45);
  }

  .badge.poisoned {
    color: #cfe87a;
    border-color: rgba(160, 200, 60, 0.5);
  }

  .badge.grilling {
    color: #f4d08a;
  }
</style>

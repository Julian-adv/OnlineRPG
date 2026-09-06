<script lang="ts">
  import { formatKg } from '../stores/inventoryStore'

  interface Props {
    current: number
    max: number
    projected?: number
    label: string
  }

  let { current, max, projected, label }: Props = $props()

  const value = $derived(projected ?? current)
  const ratio = $derived(max > 0 ? value / max : 0)
  const changed = $derived(Math.abs(value - current) > 0.001)
  const increasing = $derived(value > current)
  const remaining = $derived(Math.max(0, max - value))
</script>

<div
  class="weight-bar"
  class:heavy={ratio >= 0.9}
  class:full={ratio >= 1}
  aria-label={`${label}: ${formatKg(value)} of ${formatKg(max)} kilograms, ${formatKg(remaining)} kilograms remaining`}
  title={`${formatKg(remaining)} kg remaining`}
>
  <div class="weight-fill" style={`width: ${Math.min(ratio, 1) * 100}%`}></div>
  <span class="weight-text">
    {#if changed}
      <span class="projected-weight">{formatKg(value)}</span>
      <span
        class="weight-delta"
        class:increase={increasing}
        class:decrease={!increasing}
      >
        ({increasing ? '+' : '−'}{formatKg(Math.abs(value - current))})
      </span>
    {:else}
      <span>{formatKg(current)}</span>
    {/if}
    <span class="maximum-weight">/ {formatKg(max)} kg</span>
  </span>
</div>

<style>
  .weight-bar {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    height: 16px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid rgba(255, 255, 255, 0.15);
    box-sizing: content-box;
  }

  .weight-bar::before {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 16px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.08);
  }

  .weight-fill {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 16px;
    border-radius: 4px;
    background: linear-gradient(90deg, #1f3648, #36526b);
    transition: width 120ms ease;
  }

  .weight-bar.heavy .weight-fill {
    background: linear-gradient(90deg, #4f3a1c, #6a5732);
  }

  .weight-bar.full .weight-fill {
    background: linear-gradient(90deg, #492822, #6c3e34);
  }

  .weight-text {
    position: relative;
    z-index: 1;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: #9fb2c3;
    font-size: 11px;
    font-weight: 700;
    text-shadow: 0 0 3px rgba(0, 0, 0, 0.9);
  }

  .maximum-weight {
    color: #9fb2c3;
  }

  .projected-weight {
    color: #d8e0e7;
    font-weight: 800;
  }

  .weight-delta {
    font-weight: 800;
  }

  .weight-delta.increase {
    color: #f0c040;
  }

  .weight-delta.decrease {
    color: #8ae29a;
  }

  @media (max-width: 600px), (pointer: coarse) {
    .weight-bar {
      height: 14px;
      margin-top: 6px;
      padding-top: 6px;
    }

    .weight-fill,
    .weight-bar::before {
      height: 14px;
    }

    .weight-text {
      font-size: 10px;
    }
  }
</style>

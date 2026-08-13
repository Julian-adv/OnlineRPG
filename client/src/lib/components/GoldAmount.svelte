<script lang="ts">
  import { goldSegments } from '../utils/currency'

  const { copper }: { copper: number } = $props()

  const display = $derived(goldSegments(copper))
</script>

<span class="gold-amount"
  >{display.negative
    ? '-'
    : ''}{#each display.segments as seg, i (seg.unit)}{i > 0 ? ' ' : ''}<span
      class={seg.unit}>{seg.text}</span
    >{/each}</span
>

<style>
  .gold-amount {
    white-space: nowrap;
  }

  .gold {
    color: #ffd700;
  }

  .silver {
    color: #c8d1dc;
  }

  .copper {
    color: #c9824f;
  }
</style>

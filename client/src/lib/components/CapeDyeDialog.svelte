<script lang="ts">
  import { inventoryStore } from '../stores/inventoryStore'
  import { capeDyePreview } from '../stores/capeDyeStore'
  import { capeColorOf } from '../data/itemDefs'
  import { mountOverlay } from '../stores/overlayStack'

  interface Props {
    onConfirm: (color: string) => void
    onCancel: () => void
  }

  let { onConfirm, onCancel }: Props = $props()

  /** Dyes most players reach for, so the picker is one click for them and
   *  still free-form for everyone else. */
  const SWATCHES = [
    '#6d1720',
    '#1f3f6d',
    '#255135',
    '#4a2a5e',
    '#7a5c1e',
    '#1c1c1f',
    '#e3ded1',
    '#8c3b1a',
  ]

  const wornColor = $derived(
    capeColorOf(
      $inventoryStore.equipped.back?.item_def_id,
      $inventoryStore.equipped.back?.cape_color
    ) ?? SWATCHES[0]
  )

  let color = $state<string | null>(null)
  const chosen = $derived(color ?? wornColor)

  // The wearer sees the colour on their own cape while they pick; leaving the
  // dialog by any route puts the worn colour back.
  $effect(() => {
    capeDyePreview.set(chosen)
    return () => capeDyePreview.set(null)
  })
  $effect(() => mountOverlay('capeDye', onCancel))

  // Escape comes from the overlay stack, which owns it for every panel.
  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') onConfirm(chosen)
  }
</script>

<div
  class="dye-dialog"
  role="dialog"
  aria-label="Dye cape"
  tabindex="-1"
  onkeydown={onKeydown}
>
  <h2>Dye your cape</h2>
  <p>Pick a colour — your cape wears it until you cancel.</p>

  <div class="swatches">
    {#each SWATCHES as swatch (swatch)}
      <button
        class="swatch"
        class:selected={chosen === swatch}
        style="background: {swatch}"
        aria-label={swatch}
        onclick={() => (color = swatch)}
      ></button>
    {/each}
  </div>

  <label class="custom">
    <span>Or mix your own</span>
    <input
      type="color"
      value={chosen}
      oninput={(e) => (color = e.currentTarget.value)}
    />
  </label>

  <div class="dye-actions">
    <button class="primary" onclick={() => onConfirm(chosen)}>Dye</button>
    <button class="secondary" onclick={onCancel}>Cancel</button>
  </div>
</div>

<style>
  /* Off to the left, with no dimming backdrop: the whole point is watching
     the colour land on your own cape, and this dialog would otherwise sit on
     top of the character (screen centre) and darken them. The inventory the
     dye was used from holds the right edge. */
  .dye-dialog {
    position: fixed;
    left: 16px;
    top: 45%;
    transform: translateY(-50%);
    z-index: 40;
    width: min(300px, calc(100vw - 32px));
    padding: 20px;
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    background: rgba(16, 16, 16, 0.92);
    backdrop-filter: blur(4px);
    color: #f4f4f4;
    text-align: center;
  }

  .dye-dialog h2 {
    margin: 0 0 8px 0;
    font-size: 20px;
  }

  .dye-dialog p {
    margin: 0 0 14px 0;
    color: #d4d4d4;
    font-size: 13px;
  }

  .swatches {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
    margin-bottom: 14px;
  }

  .swatch {
    height: 34px;
    border-radius: 6px;
    border: 2px solid rgba(255, 255, 255, 0.2);
    cursor: pointer;
  }

  .swatch.selected {
    border-color: #f4f4f4;
  }

  .custom {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 16px;
    font-size: 13px;
    color: #b9b9b9;
  }

  .custom input {
    width: 56px;
    height: 30px;
    padding: 0;
    border: 1px solid rgba(255, 255, 255, 0.25);
    border-radius: 6px;
    background: transparent;
    cursor: pointer;
  }

  .dye-actions {
    display: flex;
    gap: 10px;
    justify-content: center;
  }

  .dye-actions button {
    flex: 1;
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    cursor: pointer;
  }

  .primary {
    background: #3c6e3c;
    color: #fff;
  }

  .secondary {
    background: rgba(255, 255, 255, 0.08);
    color: #f4f4f4;
  }
</style>

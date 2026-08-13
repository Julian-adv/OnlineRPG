<script lang="ts">
  import { tick } from 'svelte'
  import GoldAmount from './GoldAmount.svelte'
  import { formatGold, parseGold } from '../utils/currency'
  import { playerGold } from '../stores/inventoryStore'
  import { mountOverlay } from '../stores/overlayStack'

  interface Props {
    ownerName: string
    onConfirm: (copper: number) => void
    onCancel: () => void
  }

  let { ownerName, onConfirm, onCancel }: Props = $props()

  /** Copper presets: a handful of coins, a silver, five silver. */
  const PRESETS = [10, 50, 100, 500]

  let text = $state(formatGold(10))
  let inputEl = $state<HTMLInputElement | null>(null)

  $effect(() => mountOverlay('tipHat', onCancel))

  $effect(() => {
    tick().then(() => {
      inputEl?.focus()
      inputEl?.select()
    })
  })

  const wallet = $derived($playerGold)
  // Typing is never rewritten under the caret — the read-out above the field
  // shows what the half-typed text currently means.
  const amount = $derived(parseGold(text))
  const valid = $derived(amount !== null && amount >= 1 && amount <= wallet)

  function confirm() {
    if (!valid) return
    onConfirm(amount!)
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') confirm()
    else if (e.key === 'Escape') onCancel()
  }
</script>

<div class="tip-backdrop">
  <div
    class="tip-dialog"
    role="dialog"
    aria-modal="true"
    aria-label="Tip hat"
    tabindex="-1"
    onkeydown={onKeydown}
  >
    <h2>Tip {ownerName}</h2>
    <p>How much would you like to tip?</p>

    <div class="presets">
      {#each PRESETS as preset (preset)}
        <button class="preset" onclick={() => (text = formatGold(preset))}>
          <GoldAmount copper={preset} />
        </button>
      {/each}
    </div>

    <div class="readout" aria-live="polite">
      {#if amount === null}
        <span class="invalid">Enter an amount like 1g 20s 30c</span>
      {:else if amount > wallet}
        <span class="invalid">You only have <GoldAmount copper={wallet} /></span
        >
      {:else}
        <GoldAmount copper={amount} />
      {/if}
    </div>

    <input bind:this={inputEl} type="text" bind:value={text} />
    <div class="wallet">
      You have <GoldAmount copper={wallet} />
    </div>

    <div class="tip-actions">
      <button class="primary" disabled={!valid} onclick={confirm}>Tip</button>
      <button class="secondary" onclick={onCancel}>Cancel</button>
    </div>
  </div>
</div>

<style>
  .tip-backdrop {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.45);
    z-index: 30;
  }

  .tip-dialog {
    width: min(340px, calc(100vw - 32px));
    padding: 20px;
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    background: rgba(16, 16, 16, 0.95);
    color: #f4f4f4;
    text-align: center;
  }

  .tip-dialog h2 {
    margin: 0 0 8px 0;
    font-size: 20px;
  }

  .tip-dialog p {
    margin: 0 0 14px 0;
    color: #d4d4d4;
  }

  .presets {
    display: flex;
    gap: 6px;
    justify-content: center;
    margin-bottom: 10px;
  }

  .preset {
    flex: 1;
    padding: 6px 4px;
    border-radius: 6px;
    border: 1px solid rgba(255, 255, 255, 0.2);
    background: rgba(255, 255, 255, 0.06);
    color: inherit;
    cursor: pointer;
  }

  .preset:hover {
    background: rgba(255, 255, 255, 0.14);
  }

  .readout {
    margin-bottom: 6px;
    font-size: 18px;
    min-height: 24px;
  }

  .invalid {
    font-size: 13px;
    color: #d98b8b;
  }

  input {
    width: 100%;
    padding: 8px;
    border-radius: 6px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    background: rgba(0, 0, 0, 0.4);
    color: #f4f4f4;
    text-align: center;
    font-size: 16px;
  }

  .wallet {
    margin: 8px 0 16px 0;
    font-size: 13px;
    color: #b9b9b9;
  }

  .tip-actions {
    display: flex;
    gap: 10px;
    justify-content: center;
  }

  .tip-actions button {
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

  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .secondary {
    background: rgba(255, 255, 255, 0.08);
    color: #f4f4f4;
  }
</style>

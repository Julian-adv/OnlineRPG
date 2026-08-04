<script lang="ts">
  import { tick } from 'svelte'

  interface Props {
    visible: boolean
    itemName: string
    icon: string
    max: number
    onConfirm: (qty: number) => void
    onCancel: () => void
  }

  let { visible, itemName, icon, max, onConfirm, onCancel }: Props = $props()

  let qty = $state(1)
  let inputEl = $state<HTMLInputElement | null>(null)

  $effect(() => {
    if (!visible) return
    qty = max
    tick().then(() => {
      if (!visible) return
      inputEl?.focus()
      inputEl?.select()
    })
  })

  function clamp(n: number): number {
    if (!Number.isFinite(n)) return 1
    return Math.min(max, Math.max(1, Math.round(n)))
  }

  function onInput(e: Event) {
    const value = Number((e.target as HTMLInputElement).value)
    qty = clamp(value)
  }

  // +/- wraps between 1 and max; typed values clamp.
  function step(delta: number) {
    const zeroBased = qty - 1 + delta
    qty = (((zeroBased % max) + max) % max) + 1
  }

  function confirm() {
    onConfirm(clamp(qty))
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') confirm()
    else if (e.key === 'Escape') onCancel()
  }
</script>

{#if visible}
  <div
    class="quantity-overlay"
    role="dialog"
    aria-label="Choose quantity"
    tabindex="-1"
    onkeydown={onKeydown}
  >
    <div class="quantity-popup">
      <div class="popup-header">
        <img class="item-icon" src="/items/{icon}" alt="" draggable="false" />
        <span class="item-name">{itemName}</span>
      </div>
      <div class="qty-row">
        <button class="step-btn" onclick={() => step(-1)}>−</button>
        <input
          bind:this={inputEl}
          class="qty-input"
          type="number"
          min="1"
          {max}
          value={qty}
          oninput={onInput}
        />
        <button class="step-btn" onclick={() => step(1)}>+</button>
      </div>
      <div class="max-note">of {max}</div>
      <div class="popup-actions">
        <button class="cancel-btn" onclick={onCancel}>Cancel</button>
        <button class="confirm-btn" onclick={confirm}>Confirm</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .quantity-overlay {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.35);
    pointer-events: auto;
  }

  .quantity-popup {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
    padding: 14px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 10px;
    background: rgba(6, 10, 14, 0.95);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    min-width: 200px;
  }

  .popup-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .item-icon {
    width: 28px;
    height: 28px;
    image-rendering: pixelated;
    flex-shrink: 0;
  }

  .item-name {
    font-weight: 700;
    color: #f0c040;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .qty-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .step-btn {
    width: 32px;
    height: 32px;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 4px;
    color: #e6edf3;
    font-size: 16px;
    font-weight: 700;
    cursor: pointer;
    flex-shrink: 0;
  }

  .step-btn:hover {
    background: rgba(255, 255, 255, 0.16);
  }

  .qty-input {
    flex: 1;
    min-width: 0;
    text-align: center;
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 4px;
    color: #e6edf3;
    font-family: inherit;
    font-size: 14px;
    padding: 6px 4px;
  }

  .max-note {
    text-align: center;
    color: #6b7d8d;
  }

  .popup-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }

  .cancel-btn,
  .confirm-btn {
    border-radius: 4px;
    padding: 6px 14px;
    font-family: inherit;
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
  }

  .cancel-btn {
    background: none;
    border: 1px solid rgba(255, 255, 255, 0.2);
    color: #9fb2c3;
  }

  .cancel-btn:hover {
    color: #fff;
  }

  .confirm-btn {
    background: rgba(60, 90, 60, 0.85);
    color: #d6f0d6;
    border: 1px solid rgba(140, 220, 140, 0.35);
  }

  .confirm-btn:hover {
    background: rgba(80, 120, 80, 0.95);
    color: #fff;
  }

  @media (max-width: 600px), (pointer: coarse) {
    .step-btn {
      width: 40px;
      height: 40px;
    }
  }
</style>

<script lang="ts">
  import { tick } from 'svelte'
  import { formatGold, parseGold } from '../utils/currency'
  import GoldAmount from './GoldAmount.svelte'

  let {
    deposit,
    max,
    onconfirm,
    oncancel,
  }: {
    deposit: boolean
    max: number
    onconfirm: (amount: number) => void
    oncancel: () => void
  } = $props()
  let text = $state('')
  let input: HTMLInputElement
  const amount = $derived(parseGold(text))
  const valid = $derived(
    amount !== null &&
      Number.isSafeInteger(amount) &&
      amount > 0 &&
      amount <= max
  )
  $effect(() => {
    tick().then(() => input?.focus())
  })
  function confirm() {
    if (valid && amount !== null) onconfirm(amount)
  }
</script>

<div
  class="overlay"
  role="dialog"
  aria-modal="true"
  aria-label={deposit ? 'Deposit gold' : 'Withdraw gold'}
  tabindex="-1"
  onkeydown={(event) => {
    event.stopPropagation()
    if (event.key === 'Escape') oncancel()
  }}
>
  <form
    onsubmit={(event) => {
      event.preventDefault()
      confirm()
    }}
  >
    <strong
      >{deposit
        ? 'Deposit into tax account'
        : 'Withdraw from tax account'}</strong
    >
    <label for="land-gold-amount">How much gold?</label>
    <input
      id="land-gold-amount"
      bind:this={input}
      bind:value={text}
      placeholder="1g 20s 30c"
      autocomplete="off"
    />
    <p>Available: <GoldAmount copper={max} /></p>
    <p>Use g, s, c. A number without a unit means copper.</p>
    {#if text && !valid}<p class="error">
        Enter an amount between 1c and {formatGold(max)}.
      </p>{/if}
    {#if deposit}<p>
        Overdue accounts recover automatically when the balance covers all
        missed tax plus one month.
      </p>{/if}
    <div class="actions">
      <button type="button" onclick={oncancel}>Cancel</button>
      <button type="submit" disabled={!valid}
        >{deposit ? 'Deposit' : 'Withdraw'}</button
      >
    </div>
  </form>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: grid;
    place-items: center;
    background: #0008;
  }
  form {
    width: min(320px, 90vw);
    box-sizing: border-box;
    padding: 20px;
    border: 1px solid #cbb77866;
    border-radius: 8px;
    background: #18232e;
    color: #dfd9c9;
    font: 13px/1.5 sans-serif;
  }
  label {
    display: block;
    margin-top: 16px;
  }
  input {
    box-sizing: border-box;
    width: 100%;
    margin-top: 6px;
    padding: 8px;
    color: #fff;
    background: #0004;
    border: 1px solid #ffffff44;
    border-radius: 4px;
  }
  p {
    color: #adbdca;
    font-size: 12px;
  }
  .error {
    color: #efae87;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  button {
    padding: 7px 12px;
    border: 1px solid #ffffff44;
    border-radius: 4px;
    color: inherit;
    background: #38513d;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>

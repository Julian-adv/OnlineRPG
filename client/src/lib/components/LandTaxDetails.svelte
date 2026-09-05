<script lang="ts">
  import {
    landAccount,
    landAccountError,
    landTransferPending,
  } from '../stores/landAccountStore'
  import GoldAmount from './GoldAmount.svelte'

  let { onwithdraw }: { onwithdraw: () => void } = $props()
  const account = $derived($landAccount)
  const status = $derived.by(() => {
    if (!account?.plots) return 'No estate'
    if (account.missed >= 6) return 'Foreclosure due'
    if (account.missed >= 4) return 'Ruined'
    if (account.missed >= 2) return 'Abandoned'
    if (account.missed === 1) return 'Grace period'
    return 'Current'
  })
  const dueIn = $derived.by(() => {
    const hours = Math.ceil((account?.due_in_seconds ?? 0) / 3600)
    return hours >= 24
      ? `${Math.floor(hours / 24)}d ${hours % 24}h`
      : `${hours}h`
  })
</script>

<section class="tax-details" aria-label="Tax account">
  <button
    class="account-balance"
    onclick={onwithdraw}
    disabled={!account?.treasury || $landTransferPending}
    title="Withdraw gold from your tax account"
  >
    <span>Tax account</span>
    <GoldAmount copper={account?.treasury ?? 0} />
  </button>
  {#if $landAccountError}
    <p class="overdue" role="alert">{$landAccountError}</p>
  {/if}
  {#if !account}
    {#if !$landAccountError}<p>Loading tax account…</p>{/if}
  {:else if account.plots === 0}
    <p>Claim a homestead with a Land Deed to open your tax account.</p>
    <p>No tax is due.</p>
  {:else}
    <p class="hint">Click the account to withdraw.</p>
    <div class="tax-row">
      <span>{account.plots} plot{account.plots === 1 ? '' : 's'} · monthly</span
      ><GoldAmount copper={account.monthly_tax} />
    </div>
    <div class="tax-row">
      <span>Next tax</span><GoldAmount copper={account.next_tax} />
    </div>
    <p>
      Year {account.next_due.year}, Month {account.next_due.month}, day 1 ·
      00:00<br />
      <span class="hint">In about {dueIn} real time</span>
    </p>
    {#if account.free_months > 0}<p>
        Next {account.free_months} tax payment{account.free_months === 1
          ? ''
          : 's'} waived.
      </p>{/if}
    {#if account.treasury < account.next_tax}<p class="overdue">
        Insufficient funds for the next payment.
      </p>{/if}
    <div class="status" class:overdue={account.missed > 0}>
      <strong>Status · {status}</strong>
      {#if account.missed > 0}
        <p>
          {account.missed} consecutive missed payment{account.missed === 1
            ? ''
            : 's'}.
        </p>
        <p>
          No new construction or furniture placement. Deposits and withdrawals
          remain available.
        </p>
        {#if account.missed >= 2}<p>
            Doors unlock; furniture interactions are disabled.
          </p>{/if}
        {#if account.missed >= 4}<p>The estate is in the ruin stage.</p>{/if}
        <p>
          At 6 missed payments, land is subject to release and houses and
          furniture to removal. Furniture is returned to its owner.
        </p>
        <div class="tax-row">
          <span>Restore account</span><GoldAmount
            copper={account.recovery_cost}
          />
        </div>
        <p>
          Deposit enough to cover all missed tax plus one month. This is charged
          immediately and the next payment is waived. Partial funding does not
          clear overdue status.
        </p>
        <div class="tax-row">
          <span>Additional deposit needed</span><GoldAmount
            copper={Math.max(0, account.recovery_cost - account.treasury)}
          />
        </div>
      {:else}<p>Your account is in good standing.</p>{/if}
    </div>
  {/if}
</section>

<style>
  .tax-details {
    border-top: 1px solid #ffffff26;
    margin-top: 12px;
    padding-top: 10px;
    color: #afbfcc;
    font-size: 12px;
    line-height: 1.5;
  }
  .account-balance {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 9px;
    color: #e7d7ae;
    background: #cbb77812;
    border: 1px solid #cbb77855;
    border-radius: 4px;
    font: inherit;
    font-weight: bold;
    cursor: pointer;
  }
  .account-balance:disabled {
    cursor: default;
  }
  .account-balance:hover:not(:disabled) {
    background: #cbb77825;
  }
  .tax-row {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    margin-top: 6px;
  }
  p {
    margin: 6px 0;
  }
  .hint {
    color: #6b7d8d;
  }
  .status {
    border-top: 1px solid #ffffff26;
    margin-top: 10px;
    padding-top: 10px;
  }
  .overdue {
    color: #efae87;
  }
</style>

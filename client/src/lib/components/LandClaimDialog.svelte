<script lang="ts">
  import { landClaimDialog } from '../stores/landClaimStore'
  import { mountOverlay } from '../stores/overlayStack'
  import { networkManager } from '../network/socket'

  function close() {
    if ($landClaimDialog?.status !== 'pending') landClaimDialog.set(null)
  }

  function confirm() {
    const claim = $landClaimDialog
    if (!claim || claim.status !== 'confirm' || claim.refreshing) return
    landClaimDialog.set({ ...claim, status: 'pending' })
    networkManager.sendLandClaim(claim)
  }

  $effect(() => mountOverlay('landClaim', close))
</script>

{#if $landClaimDialog}
  <div
    class="land-dialog"
    role="dialog"
    aria-modal="false"
    aria-labelledby="land-claim-title"
    tabindex="-1"
  >
    <h2 id="land-claim-title">
      {$landClaimDialog.status === 'claimed'
        ? 'Land claimed'
        : $landClaimDialog.status === 'rejected'
          ? 'Cannot claim this plot'
          : 'Claim this plot?'}
    </h2>
    <p class="plot">32 × 32 m · 1,024 m²</p>
    {#if $landClaimDialog.refreshing}
      <p role="status">Checking this plot…</p>
    {:else if $landClaimDialog.status === 'claimed'}
      <p>This plot is now part of your homestead.</p>
      <p>One Land Deed was consumed.</p>
    {:else if $landClaimDialog.status === 'rejected'}
      <p role="alert">{$landClaimDialog.reason}</p>
      <p>Your Land Deed was not consumed.</p>
    {:else}
      <p>Claim the highlighted plot as your homestead?</p>
      <p>
        Requires level 10 and unclaimed homestead land. Expansions must share an
        edge with your land.
      </p>
      <p>One Land Deed will be consumed on success.</p>
    {/if}
    <div class="actions">
      {#if $landClaimDialog.status === 'confirm' || $landClaimDialog.status === 'pending'}
        <button
          class="primary"
          onclick={confirm}
          disabled={$landClaimDialog.status === 'pending' ||
            $landClaimDialog.refreshing}
        >
          {$landClaimDialog.refreshing
            ? 'Checking…'
            : $landClaimDialog.status === 'pending'
              ? 'Claiming…'
              : 'Claim plot'}
        </button>
        <button onclick={close} disabled={$landClaimDialog.status === 'pending'}
          >Cancel</button
        >
      {:else}
        <button onclick={close}>Close</button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .land-dialog {
    position: fixed;
    left: 16px;
    top: 45%;
    transform: translateY(-50%);
    z-index: 40;
    width: min(300px, calc(100vw - 32px));
    box-sizing: border-box;
    padding: 16px;
    border-radius: 12px;
    border: 1px solid #aa915b;
    background: rgba(16, 20, 16, 0.95);
    color: #f4f4f4;
  }
  h2 {
    margin: 0 0 8px;
    font-size: 20px;
  }
  p {
    font-size: 13px;
    line-height: 1.5;
    color: #d4d4d4;
  }
  .plot {
    color: #ebcb83;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 18px;
  }
  button {
    padding: 7px 12px;
    font-size: 13px;
    border: 1px solid #666;
    border-radius: 6px;
    background: #333;
    color: white;
    cursor: pointer;
  }
  button.primary {
    background: #456334;
    border-color: #799b59;
  }
  button:disabled {
    opacity: 0.6;
    cursor: wait;
  }
</style>

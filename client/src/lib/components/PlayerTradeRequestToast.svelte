<script lang="ts">
  import {
    pendingTradeRequests,
    TRADE_REQUEST_TTL_MS,
  } from '../stores/playerTradeStore'
  import { networkManager } from '../network/socket'
  import QueuedConsentToast from './QueuedConsentToast.svelte'
</script>

<QueuedConsentToast
  queue={pendingTradeRequests}
  ttlMs={TRADE_REQUEST_TTL_MS}
  label="Trade request"
  top="28%"
  accent="#e0c070"
  acceptLabel="Trade"
  declineLabel="Decline"
  respond={(request, accept) =>
    networkManager.sendPlayerTradeRespond(request.requesterId, accept)}
>
  {#snippet children(request)}
    <strong>{request.requesterName}</strong> wants to trade with you
  {/snippet}
</QueuedConsentToast>

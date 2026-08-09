<script lang="ts">
  import {
    pendingFriendRequests,
    FRIEND_REQUEST_TTL_MS,
  } from '../stores/friendStore'
  import { networkManager } from '../network/socket'
  import QueuedConsentToast from './QueuedConsentToast.svelte'
</script>

<QueuedConsentToast
  queue={pendingFriendRequests}
  ttlMs={FRIEND_REQUEST_TTL_MS}
  label="Friend request"
  top="32%"
  accent="#8fe08f"
  acceptLabel="Accept"
  declineLabel="Decline"
  respond={(request, accept) =>
    networkManager.sendFriendRespond(request.requesterId, accept)}
>
  {#snippet children(request)}
    <strong>{request.requesterName}</strong> wants to be friends
  {/snippet}
</QueuedConsentToast>

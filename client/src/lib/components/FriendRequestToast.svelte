<script lang="ts">
  import {
    pendingFriendRequests,
    FRIEND_REQUEST_TTL_MS,
    type PendingFriendRequest,
  } from '../stores/friendStore'
  import { networkManager } from '../network/socket'
  import ConsentToast from './ConsentToast.svelte'

  /** Oldest first — the queue keeps a flood from swapping the name under
   *  the user's click or burying an earlier legitimate request. */
  const request = $derived($pendingFriendRequests[0] ?? null)
  const queued = $derived(Math.max(0, $pendingFriendRequests.length - 1))

  function dismiss(request: PendingFriendRequest) {
    pendingFriendRequests.update((queue) => queue.filter((r) => r !== request))
  }

  function respond(request: PendingFriendRequest, accept: boolean) {
    networkManager.sendFriendRespond(request.requesterId, accept)
    dismiss(request)
  }

  $effect(() => {
    if (!request) return
    const timer = setTimeout(
      () => dismiss(request),
      Math.max(0, request.offeredAt + FRIEND_REQUEST_TTL_MS - Date.now())
    )
    return () => clearTimeout(timer)
  })
</script>

{#if request}
  <ConsentToast
    label="Friend request"
    top="32%"
    accent="#8fe08f"
    acceptLabel="Accept"
    declineLabel="Decline"
    onaccept={() => respond(request, true)}
    ondecline={() => respond(request, false)}
    {queued}
    gaugeDurationMs={FRIEND_REQUEST_TTL_MS}
    gaugeStartAt={request.offeredAt}
  >
    <strong>{request.requesterName}</strong> wants to be friends
  </ConsentToast>
{/if}

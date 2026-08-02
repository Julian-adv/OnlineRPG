<script lang="ts">
  import {
    pendingPartyInvites,
    INVITE_TTL_MS,
    type PendingPartyInvite,
  } from '../stores/partyStore'
  import { networkManager } from '../network/socket'
  import ConsentToast from './ConsentToast.svelte'

  /** Oldest first — the queue keeps a flood from swapping the name under
   *  the user's click or burying an earlier legitimate invite. */
  const invite = $derived($pendingPartyInvites[0] ?? null)
  const queued = $derived(Math.max(0, $pendingPartyInvites.length - 1))

  function dismiss(invite: PendingPartyInvite) {
    pendingPartyInvites.update((queue) => queue.filter((i) => i !== invite))
  }

  function respond(invite: PendingPartyInvite, accept: boolean) {
    networkManager.sendPartyRespond(invite.inviterId, accept)
    dismiss(invite)
  }

  $effect(() => {
    if (!invite) return
    const timer = setTimeout(
      () => dismiss(invite),
      Math.max(0, invite.offeredAt + INVITE_TTL_MS - Date.now())
    )
    return () => clearTimeout(timer)
  })
</script>

{#if invite}
  <ConsentToast
    label="Party invite"
    top="20%"
    accent="#7ec8ff"
    acceptLabel="Join"
    declineLabel="Decline"
    onaccept={() => respond(invite, true)}
    ondecline={() => respond(invite, false)}
    {queued}
    gaugeDurationMs={INVITE_TTL_MS}
    gaugeStartAt={invite.offeredAt}
  >
    <strong>{invite.inviterName}</strong> invites you to a party
  </ConsentToast>
{/if}

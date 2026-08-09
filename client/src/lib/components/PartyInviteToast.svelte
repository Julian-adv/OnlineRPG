<script lang="ts">
  import { pendingPartyInvites, INVITE_TTL_MS } from '../stores/partyStore'
  import { networkManager } from '../network/socket'
  import QueuedConsentToast from './QueuedConsentToast.svelte'
</script>

<QueuedConsentToast
  queue={pendingPartyInvites}
  ttlMs={INVITE_TTL_MS}
  label="Party invite"
  top="20%"
  accent="#7ec8ff"
  acceptLabel="Join"
  declineLabel="Decline"
  respond={(invite, accept) =>
    networkManager.sendPartyRespond(invite.inviterId, accept)}
>
  {#snippet children(invite)}
    <strong>{invite.inviterName}</strong> invites you to a party
  {/snippet}
</QueuedConsentToast>

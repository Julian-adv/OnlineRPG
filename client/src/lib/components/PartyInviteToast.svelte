<script lang="ts">
  import {
    pendingPartyInvites,
    type PendingPartyInvite,
  } from '../stores/partyStore'
  import { networkManager } from '../network/socket'

  /** Mirrors the server-side invite TTL. */
  const INVITE_TTL_MS = 30_000

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
  <div class="party-invite" role="alertdialog" aria-label="Party invite">
    <span class="invite-text">
      <strong>{invite.inviterName}</strong> invites you to a party
      {#if queued > 0}<span class="queued">(+{queued} waiting)</span>{/if}
    </span>
    <button class="accept-btn" onclick={() => respond(invite, true)}>
      Join
    </button>
    <button class="decline-btn" onclick={() => respond(invite, false)}>
      Decline
    </button>
  </div>
{/if}

<style>
  .party-invite {
    position: fixed;
    left: 50%;
    top: 20%;
    transform: translateX(-50%);
    z-index: 44;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 10px;
    background: rgba(6, 10, 14, 0.88);
    backdrop-filter: blur(4px);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    pointer-events: auto;
  }

  .invite-text strong {
    color: #7ec8ff;
  }

  .queued {
    color: #9fb2c3;
  }

  .accept-btn,
  .decline-btn {
    border-radius: 4px;
    padding: 4px 10px;
    font-family: inherit;
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  .accept-btn {
    background: rgba(60, 90, 60, 0.85);
    color: #d6f0d6;
    border: 1px solid rgba(140, 220, 140, 0.35);
  }

  .accept-btn:hover {
    background: rgba(80, 120, 80, 0.95);
    color: #fff;
  }

  .decline-btn {
    background: none;
    color: #9fb2c3;
    border: 1px solid rgba(255, 255, 255, 0.18);
  }

  .decline-btn:hover {
    color: #fff;
    border-color: rgba(255, 255, 255, 0.4);
  }

  @media (pointer: coarse) {
    .accept-btn,
    .decline-btn {
      min-height: 32px;
    }
  }
</style>

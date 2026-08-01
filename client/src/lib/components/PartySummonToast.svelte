<script lang="ts">
  import {
    pendingPartySummons,
    SUMMON_TTL_MS,
    type PendingPartySummon,
  } from '../stores/partyStore'
  import { networkManager } from '../network/socket'

  /** Oldest first, same queue discipline as PartyInviteToast. */
  const summon = $derived($pendingPartySummons[0] ?? null)
  const queued = $derived(Math.max(0, $pendingPartySummons.length - 1))

  function dismiss(summon: PendingPartySummon) {
    pendingPartySummons.update((queue) => queue.filter((s) => s !== summon))
  }

  function respond(summon: PendingPartySummon, accept: boolean) {
    networkManager.sendPartySummonRespond(summon.casterId, accept)
    // An accept keeps the toast up: the server refuses mid-combat accepts
    // and the pending summon survives for a retry, so the gauge must too. A
    // successful one clears it via the player's own PlayerTeleported.
    if (!accept) dismiss(summon)
  }

  $effect(() => {
    if (!summon) return
    const t = setTimeout(
      () => dismiss(summon),
      Math.max(0, summon.offeredAt + SUMMON_TTL_MS - Date.now())
    )
    return () => clearTimeout(t)
  })
</script>

{#if summon}
  <div class="party-summon" role="alertdialog" aria-label="Party summon">
    <div class="summon-row">
      <span class="summon-text">
        <strong>{summon.casterName}</strong> calls you to their side
        {#if queued > 0}<span class="queued">(+{queued} waiting)</span>{/if}
      </span>
      <button class="accept-btn" onclick={() => respond(summon, true)}>
        Answer
      </button>
      <button class="decline-btn" onclick={() => respond(summon, false)}>
        Ignore
      </button>
    </div>
    <div class="gauge">
      <!-- CSS-driven drain: a negative delay skips the elapsed part, and
           {#key} restarts the animation when the head summon changes. -->
      {#key summon}
        <div
          class="gauge-fill"
          style="animation-duration: {SUMMON_TTL_MS}ms; animation-delay: {summon.offeredAt -
            Date.now()}ms"
        ></div>
      {/key}
    </div>
  </div>
{/if}

<style>
  .party-summon {
    position: fixed;
    left: 50%;
    top: 26%;
    transform: translateX(-50%);
    z-index: 44;
    padding: 8px 12px 6px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 10px;
    background: rgba(6, 10, 14, 0.88);
    backdrop-filter: blur(4px);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    pointer-events: auto;
  }

  .summon-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .summon-text strong {
    color: #c8a2ff;
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

  .gauge {
    margin-top: 6px;
    height: 3px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.12);
    overflow: hidden;
  }

  .gauge-fill {
    height: 100%;
    border-radius: 2px;
    background: #c8a2ff;
    transform-origin: left;
    animation: drain linear forwards;
  }

  @keyframes drain {
    from {
      transform: scaleX(1);
    }
    to {
      transform: scaleX(0);
    }
  }

  @media (pointer: coarse) {
    .accept-btn,
    .decline-btn {
      min-height: 32px;
    }
  }
</style>

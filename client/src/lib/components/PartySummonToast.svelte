<script lang="ts">
  import {
    pendingPartySummons,
    SUMMON_TTL_MS,
    type PendingPartySummon,
  } from '../stores/partyStore'
  import { networkManager } from '../network/socket'
  import ConsentToast from './ConsentToast.svelte'

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
  <ConsentToast
    label="Party summon"
    top="26%"
    accent="#c8a2ff"
    acceptLabel="Answer"
    declineLabel="Ignore"
    onaccept={() => respond(summon, true)}
    ondecline={() => respond(summon, false)}
    {queued}
    gaugeDurationMs={SUMMON_TTL_MS}
    gaugeStartAt={summon.offeredAt}
  >
    <strong>{summon.casterName}</strong> calls you to their side
  </ConsentToast>
{/if}

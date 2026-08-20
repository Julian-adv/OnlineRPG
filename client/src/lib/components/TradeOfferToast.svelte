<script lang="ts">
  import {
    pendingTradeOffer,
    acceptTradeOffer,
    declineTradeOffer,
    shopDeals,
    hasLiveDeal,
    type PendingTradeOffer,
  } from '../stores/tradeStore'
  import { networkManager } from '../network/socket'
  import ConsentToast from './ConsentToast.svelte'

  /** Offers go stale fast (the NPC may wander out of trade range). */
  const OFFER_TTL_MS = 30_000

  const offer = $derived($pendingTradeOffer)

  // A haggled price is only drawn inside the window this toast asks to open.
  const withDeal = $derived(
    offer ? hasLiveDeal($shopDeals, offer.session.merchantPlayerId) : false
  )

  function accept(offer: PendingTradeOffer) {
    acceptTradeOffer(offer)
    // Register the open window server-side so the NPC holds position while we
    // shop (an NPC-pushed offer doesn't register until the player opts in).
    networkManager.sendOpenShop(offer.session.merchantPlayerId)
  }

  function decline(offer: PendingTradeOffer) {
    declineTradeOffer()
    // Tell the NPC, so it lets trading rest instead of offering again.
    networkManager.sendDeclineTrade(offer.session.merchantPlayerId)
  }

  $effect(() => {
    if (!offer) return
    // Letting the offer expire is a decline too — an unanswered toast means
    // the player is not interested (or not there); either way the NPC
    // should stop pushing.
    const timer = setTimeout(
      () => decline(offer),
      Math.max(0, offer.offeredAt + OFFER_TTL_MS - Date.now())
    )
    return () => clearTimeout(timer)
  })
</script>

{#if offer}
  <ConsentToast
    label="Trade offer"
    top="14%"
    accent="#f0c040"
    acceptLabel="Open"
    declineLabel="Not now"
    onaccept={() => accept(offer)}
    ondecline={() => decline(offer)}
  >
    <strong>{offer.session.merchantName}</strong> wants to trade with you
    {#if withDeal}<span class="deal-note">— holding a price for you</span>{/if}
  </ConsentToast>
{/if}

<style>
  .deal-note {
    color: var(--accent);
    font-weight: 600;
  }
</style>

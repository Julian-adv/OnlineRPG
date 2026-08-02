<script lang="ts">
  import {
    pendingTradeOffer,
    acceptTradeOffer,
    declineTradeOffer,
    type PendingTradeOffer,
  } from '../stores/tradeStore'
  import { networkManager } from '../network/socket'
  import ConsentToast from './ConsentToast.svelte'

  /** Offers go stale fast (the NPC may wander out of trade range). */
  const OFFER_TTL_MS = 30_000

  const offer = $derived($pendingTradeOffer)

  function accept(offer: PendingTradeOffer) {
    acceptTradeOffer(offer)
    // Register the open window server-side so the NPC holds position while we
    // shop (an NPC-pushed offer doesn't register until the player opts in).
    networkManager.sendOpenShop(offer.session.merchantPlayerId)
  }

  $effect(() => {
    if (!offer) return
    const timer = setTimeout(
      declineTradeOffer,
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
    ondecline={declineTradeOffer}
  >
    <strong>{offer.session.merchantName}</strong> wants to trade with you
  </ConsentToast>
{/if}

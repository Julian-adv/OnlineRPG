<script lang="ts">
  import { onMount } from 'svelte'
  import { playerGold } from '../stores/inventoryStore'
  import {
    teleportGateBusy,
    teleportGateSession,
  } from '../stores/teleportGateStore'
  import { networkManager } from '../network/socket'
  import GoldAmount from './GoldAmount.svelte'

  const session = $derived($teleportGateSession)

  function close() {
    if ($teleportGateBusy) return
    teleportGateSession.set(null)
  }

  function closeFromBackdrop(event: MouseEvent) {
    if (event.target === event.currentTarget) close()
  }

  function travel(destinationGateId: string) {
    if (!session || $teleportGateBusy) return
    teleportGateBusy.set(true)
    networkManager.sendUseTeleportGate(session.gateId, destinationGateId)
  }

  onMount(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  })
</script>

{#if session}
  <div class="backdrop" role="presentation" onclick={closeFromBackdrop}>
    <div
      class="gate-dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="gate-title"
    >
      <header>
        <div>
          <p class="eyebrow">Ancient town gate</p>
          <h2 id="gate-title">{session.townName}</h2>
        </div>
        <button
          class="close"
          onclick={close}
          disabled={$teleportGateBusy}
          aria-label="Close"
        >
          ×
        </button>
      </header>

      <div class="warning">
        <strong>Gatekeeper’s notice</strong>
        <span
          >Fares increase with distance. There is a {(
            session.misfireChanceBps / 100
          ).toFixed(2)}% chance of being thrown to a random point on land, at
          sea, or inside a dungeon. Carry a Scroll of Return.</span
        >
      </div>

      <div class="wallet">Wallet: <GoldAmount copper={$playerGold} /></div>

      <div class="destinations">
        {#each session.destinations as destination (destination.gateId)}
          {@const affordable = $playerGold >= destination.fare}
          <button
            class="destination"
            class:unaffordable={!affordable}
            disabled={!affordable || $teleportGateBusy}
            onclick={() => travel(destination.gateId)}
          >
            <span class="town">{destination.townName}</span>
            <span class="distance"
              >{(destination.distanceM / 1000).toFixed(1)} km</span
            >
            <GoldAmount copper={destination.fare} />
          </button>
        {/each}
      </div>

      {#if $teleportGateBusy}
        <p class="status">The runes are aligning…</p>
      {:else}
        <p class="hint">
          Choose a destination. Payment is taken only if the gate activates; a
          misfire still charges the quoted fare.
        </p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 92;
    display: grid;
    place-items: center;
    padding: 20px;
    background: rgba(5, 8, 17, 0.62);
    backdrop-filter: blur(3px);
  }

  .gate-dialog {
    width: min(520px, 94vw);
    max-height: min(720px, 88vh);
    overflow: auto;
    color: #edf4ff;
    border: 1px solid rgba(126, 197, 255, 0.55);
    border-radius: 16px;
    background:
      radial-gradient(
        circle at 80% 0%,
        rgba(52, 145, 255, 0.2),
        transparent 42%
      ),
      linear-gradient(155deg, rgba(18, 27, 46, 0.98), rgba(8, 13, 25, 0.98));
    box-shadow:
      0 24px 70px rgba(0, 0, 0, 0.65),
      inset 0 0 28px rgba(66, 155, 255, 0.08);
    padding: 22px;
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
  }

  .eyebrow {
    margin: 0 0 3px;
    color: #84c8ff;
    font-size: 0.72rem;
    font-weight: 800;
    letter-spacing: 0.15em;
    text-transform: uppercase;
  }

  h2 {
    margin: 0;
    font-family: Georgia, serif;
    font-size: 1.85rem;
  }

  .close {
    width: 34px;
    height: 34px;
    border: 0;
    border-radius: 50%;
    color: #dcecff;
    background: rgba(255, 255, 255, 0.08);
    font-size: 1.45rem;
    cursor: pointer;
  }

  .warning {
    display: grid;
    gap: 4px;
    margin: 18px 0 12px;
    padding: 12px 14px;
    border-left: 3px solid #d6a646;
    border-radius: 5px;
    color: #f2dfb5;
    background: rgba(132, 86, 18, 0.18);
    font-size: 0.86rem;
    line-height: 1.45;
  }

  .wallet {
    margin-bottom: 12px;
    color: #b8c7db;
    font-size: 0.86rem;
    text-align: right;
  }

  .destinations {
    display: grid;
    gap: 8px;
  }

  .destination {
    display: grid;
    grid-template-columns: 1fr auto auto;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 12px 14px;
    border: 1px solid rgba(124, 181, 231, 0.25);
    border-radius: 9px;
    color: inherit;
    background: rgba(87, 139, 192, 0.1);
    cursor: pointer;
    text-align: left;
    transition:
      border-color 120ms ease,
      background 120ms ease,
      transform 120ms ease;
  }

  .destination:hover:not(:disabled) {
    transform: translateY(-1px);
    border-color: rgba(116, 200, 255, 0.75);
    background: rgba(62, 153, 225, 0.2);
  }

  .destination:disabled {
    cursor: not-allowed;
  }

  .destination.unaffordable {
    opacity: 0.48;
  }

  .town {
    font-family: Georgia, serif;
    font-size: 1.04rem;
    font-weight: 700;
  }

  .distance {
    color: #91a6bd;
    font-size: 0.78rem;
  }

  .status,
  .hint {
    margin: 14px 0 0;
    color: #8dbfe8;
    font-size: 0.78rem;
    text-align: center;
  }

  @media (max-width: 520px) {
    .destination {
      grid-template-columns: 1fr auto;
    }

    .distance {
      grid-column: 1;
      grid-row: 2;
    }
  }
</style>

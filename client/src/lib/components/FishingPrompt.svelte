<script lang="ts">
  // The local player's fishing status line + its two keys: SPACE hooks on a
  // bite, ESC reels in. Self-contained window listeners (like dialogs) so
  // the player-control FSM stays untouched; the server is the judge of
  // timing either way.
  import { myFishingPhase } from '../stores/fishingStore'
  import { networkManager } from '../network/socket'

  function onKeydown(event: KeyboardEvent) {
    if ($myFishingPhase === 'idle') return
    if (event.code === 'Space') {
      event.preventDefault()
      if ($myFishingPhase === 'bite') {
        networkManager.sendFishingRespond('hook')
      }
    } else if (event.code === 'Escape') {
      event.preventDefault()
      networkManager.sendFishingStop()
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if $myFishingPhase === 'casting'}
  <div class="fishing-prompt waiting">Fishing… watch the bobber</div>
{:else if $myFishingPhase === 'bite'}
  <button
    class="fishing-prompt bite"
    onclick={() => networkManager.sendFishingRespond('hook')}
  >
    ! HOOK IT — press SPACE
  </button>
{/if}

<style>
  .fishing-prompt {
    position: fixed;
    left: 50%;
    bottom: 22%;
    transform: translateX(-50%);
    padding: 8px 18px;
    border-radius: 999px;
    font-size: 15px;
    pointer-events: none;
    z-index: 30;
  }

  .waiting {
    background: rgba(12, 24, 38, 0.75);
    color: #a6c8ee;
    border: 1px solid rgba(166, 200, 238, 0.35);
  }

  .bite {
    pointer-events: auto;
    cursor: pointer;
    background: rgba(213, 73, 60, 0.92);
    color: #fff;
    border: 1px solid #f2ede2;
    font-weight: 700;
    animation: fishing-pulse 0.5s ease-in-out infinite alternate;
  }

  @keyframes fishing-pulse {
    from {
      transform: translateX(-50%) scale(1);
    }
    to {
      transform: translateX(-50%) scale(1.08);
    }
  }
</style>

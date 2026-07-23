<script lang="ts">
  // The local player's fishing HUD: status line while waiting, the hook
  // call on a bite, and the round-by-round struggle panel (tension bar,
  // fish-state prompt, countdown). Keys: SPACE = hook/reel, S = give line,
  // ESC = reel in and quit. Self-contained window listeners (dialog
  // pattern) so the player-control FSM stays untouched; the server judges
  // all timing regardless of what this UI shows.
  import { myFishingPhase, myStruggle } from '../stores/fishingStore'
  import { networkManager } from '../network/socket'

  let countdownPct = $state(100)
  let raf: number | null = null

  // Animate the countdown ring from the round's client receipt time. Purely
  // cosmetic — the authoritative deadline is server-side (plus grace), so
  // running slightly ahead of the truth only makes players early, never late.
  $effect(() => {
    const struggle = $myStruggle
    if (!struggle) {
      countdownPct = 100
      if (raf !== null) cancelAnimationFrame(raf)
      raf = null
      return
    }
    const tick = () => {
      const elapsed = performance.now() - struggle.startedAt
      countdownPct = Math.max(
        0,
        100 - (elapsed / struggle.respondWithinMs) * 100
      )
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => {
      if (raf !== null) cancelAnimationFrame(raf)
      raf = null
    }
  })

  function respond(action: 'hook' | 'reel' | 'giveline') {
    networkManager.sendFishingRespond(action)
  }

  function onKeydown(event: KeyboardEvent) {
    if ($myFishingPhase === 'idle') return
    if (event.code === 'Space') {
      event.preventDefault()
      if ($myFishingPhase === 'bite') respond('hook')
      else if ($myFishingPhase === 'struggle') respond('reel')
    } else if (event.code === 'KeyS') {
      if ($myFishingPhase === 'struggle') {
        event.preventDefault()
        respond('giveline')
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
  <button class="fishing-prompt bite" onclick={() => respond('hook')}>
    ! HOOK IT — press SPACE
  </button>
{:else if $myFishingPhase === 'struggle' && $myStruggle}
  {@const s = $myStruggle}
  <div class="struggle-panel">
    <div class="struggle-header">
      <span class="rounds">Round {s.round}/{s.totalRounds}</span>
      <span class="countdown-track"
        ><span class="countdown-fill" style={`width: ${countdownPct}%`}
        ></span></span
      >
    </div>
    {#if s.fishState === 'pulling'}
      <button
        class="struggle-action pulling"
        onclick={() => respond('giveline')}
      >
        The fish PULLS — GIVE LINE (S)
      </button>
    {:else}
      <button class="struggle-action tiring" onclick={() => respond('reel')}>
        It tires — REEL IN (SPACE)
      </button>
    {/if}
    <div
      class="tension-track"
      role="progressbar"
      aria-label="Line tension"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={s.tension}
    >
      <span
        class="tension-fill"
        class:tension-high={s.tension >= 70}
        style={`width: ${Math.min(100, s.tension)}%`}
      ></span>
    </div>
    <div class="tension-label">Line tension — snaps at 100</div>
  </div>
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

  .struggle-panel {
    position: fixed;
    left: 50%;
    bottom: 20%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: min(340px, 86vw);
    padding: 12px 16px;
    border-radius: 12px;
    background: rgba(12, 24, 38, 0.88);
    border: 1px solid rgba(166, 200, 238, 0.35);
    z-index: 30;
  }

  .struggle-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .rounds {
    color: #a6c8ee;
    font-size: 13px;
    white-space: nowrap;
  }

  .countdown-track {
    position: relative;
    flex: 1;
    height: 5px;
    border-radius: 999px;
    overflow: hidden;
    background: rgba(64, 98, 135, 0.45);
  }

  .countdown-fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: #a6c8ee;
  }

  .struggle-action {
    cursor: pointer;
    padding: 10px 12px;
    border-radius: 8px;
    font-weight: 700;
    font-size: 15px;
    color: #fff;
    border: 1px solid #f2ede2;
  }

  .struggle-action.pulling {
    background: rgba(213, 73, 60, 0.92);
  }

  .struggle-action.tiring {
    background: rgba(63, 153, 96, 0.92);
  }

  .tension-track {
    position: relative;
    height: 8px;
    border-radius: 999px;
    overflow: hidden;
    background: rgba(64, 98, 135, 0.45);
    border: 1px solid rgba(166, 200, 238, 0.25);
  }

  .tension-fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: linear-gradient(90deg, #e8c34f 0%, #d5493c 100%);
    transition: width 0.15s ease-out;
  }

  .tension-fill.tension-high {
    animation: fishing-pulse-bar 0.4s ease-in-out infinite alternate;
  }

  @keyframes fishing-pulse-bar {
    from {
      filter: brightness(1);
    }
    to {
      filter: brightness(1.35);
    }
  }

  .tension-label {
    color: rgba(166, 200, 238, 0.7);
    font-size: 11px;
    text-align: center;
  }
</style>

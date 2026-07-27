<script lang="ts">
  // Local player's fishing HUD. SPACE = hook/reel, S = give line, ESC = quit;
  // the server judges all timing regardless of what this UI shows.
  import { myFishing } from '../stores/fishingStore'
  import { networkManager } from '../network/socket'
  import { isTypingTarget } from '../utils/dom'

  function respond(action: 'hook' | 'reel' | 'giveline') {
    networkManager.sendFishingRespond(action)
  }

  function onKeydown(event: KeyboardEvent) {
    const phase = $myFishing.phase
    if (phase === 'idle') return
    if (isTypingTarget(event.target)) return
    if (event.code === 'Space') {
      event.preventDefault()
      if (phase === 'bite') respond('hook')
      else if (phase === 'struggle') respond('reel')
    } else if (event.code === 'KeyS') {
      if (phase === 'struggle') {
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

{#if $myFishing.phase === 'casting'}
  <div class="fishing-prompt waiting">Fishing… watch the bobber</div>
{:else if $myFishing.phase === 'bite'}
  <button class="fishing-prompt bite" onclick={() => respond('hook')}>
    ! HOOK IT — press SPACE
  </button>
{:else if $myFishing.phase === 'struggle'}
  {@const s = $myFishing.struggle}
  <div class="struggle-panel">
    <div class="struggle-header">
      <span class="rounds">Round {s.round}/{s.totalRounds}</span>
      <!-- Cosmetic countdown, restarted per round by the keyed block; the
           authoritative deadline is server-side. -->
      {#key s.round}
        <span class="countdown-track"
          ><span
            class="countdown-fill"
            style={`animation-duration: ${s.respondWithinMs}ms`}
          ></span></span
        >
      {/key}
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
    background: rgba(6, 10, 14, 0.75);
    color: #e6edf3;
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
    background: rgba(6, 10, 14, 0.88);
    border: 1px solid rgba(166, 200, 238, 0.35);
    z-index: 30;
  }

  .struggle-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .rounds {
    color: #e6edf3;
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
    width: 100%;
    background: #e6edf3;
    animation-name: countdown-drain;
    animation-timing-function: linear;
    animation-fill-mode: forwards;
  }

  @keyframes countdown-drain {
    from {
      width: 100%;
    }
    to {
      width: 0%;
    }
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

<script lang="ts">
  import type { Snippet } from 'svelte'
  import { PANEL_Z_CEILING } from '../stores/panelLayout'

  /** Shared shell for the accept/decline consent toasts (party invite, trade
   * offer, party summon): fixed top-center card, accent-colored name, button
   * pair, optional queue count and TTL drain gauge. Behavior (stores, network,
   * TTL dismissal) stays in each caller. */
  let {
    label,
    top,
    accent,
    acceptLabel,
    declineLabel,
    onaccept,
    ondecline,
    queued = 0,
    gaugeDurationMs = 0,
    gaugeStartAt = 0,
    children,
  }: {
    label: string
    top: string
    accent: string
    acceptLabel: string
    declineLabel: string
    onaccept: () => void
    ondecline: () => void
    queued?: number
    gaugeDurationMs?: number
    gaugeStartAt?: number
    children: Snippet
  } = $props()
</script>

<div
  class="consent-toast"
  class:with-gauge={gaugeDurationMs > 0}
  role="alertdialog"
  aria-label={label}
  style="top: {top}; --accent: {accent}; z-index: {PANEL_Z_CEILING}"
>
  <div class="toast-row">
    <span class="toast-text">
      {@render children()}
      {#if queued > 0}<span class="queued">(+{queued} waiting)</span>{/if}
    </span>
    <button class="accept-btn" onclick={onaccept}>{acceptLabel}</button>
    <button class="decline-btn" onclick={ondecline}>{declineLabel}</button>
  </div>
  {#if gaugeDurationMs > 0}
    <div class="gauge">
      <!-- CSS-driven drain: a negative delay skips the elapsed part, and
           {#key} restarts the animation when the head entry changes. -->
      {#key gaugeStartAt}
        <div
          class="gauge-fill"
          style="animation-duration: {gaugeDurationMs}ms; animation-delay: {gaugeStartAt -
            Date.now()}ms"
        ></div>
      {/key}
    </div>
  {/if}
</div>

<style>
  .consent-toast {
    position: fixed;
    left: 50%;
    transform: translateX(-50%);
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

  .consent-toast.with-gauge {
    padding-bottom: 6px;
  }

  .toast-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .toast-text :global(strong) {
    color: var(--accent);
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
    background: var(--accent);
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

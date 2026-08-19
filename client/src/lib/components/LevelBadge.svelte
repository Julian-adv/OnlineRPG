<script lang="ts">
  import { untrack } from 'svelte'
  import { Tween } from 'svelte/motion'
  import { cubicOut } from 'svelte/easing'
  import { characterPanelVisible } from '../stores/debugStore'
  import { levelProgress } from '../utils/xpProgress'

  let { level, xp }: { level: number; xp: number } = $props()

  const xpInfo = $derived(levelProgress(level, xp))
  const ring = new Tween(0, { duration: 300, easing: cubicOut })

  let pulse = $state<'gain' | 'level' | null>(null)
  let pulseTimer: ReturnType<typeof setTimeout> | undefined
  let prevLevel = level
  let prevXp = xp

  $effect(() => {
    const target = xpInfo.progress
    const leveled = level > prevLevel
    const gained = xp > prevXp
    prevLevel = level
    prevXp = xp
    untrack(() => {
      if (leveled) {
        ring
          .set(1, { duration: 350 })
          .then(() => ring.set(0, { duration: 0 }))
          .then(() => ring.set(target, { duration: 450 }))
      } else {
        ring.set(target)
      }
      if (leveled || gained) {
        pulse = leveled ? 'level' : 'gain'
        clearTimeout(pulseTimer)
        pulseTimer = setTimeout(() => (pulse = null), leveled ? 1000 : 700)
      }
    })
  })

  function toggle() {
    characterPanelVisible.update((v) => !v)
  }
</script>

<button
  type="button"
  class="level-badge"
  class:gaining={pulse === 'gain'}
  class:leveling-up={pulse === 'level'}
  style:--xp={`${ring.current * 100}%`}
  aria-label={`Level ${level}, ${xpInfo.gainedXp} of ${xpInfo.neededXp} XP (${xpInfo.percent}%). Open character panel`}
  onclick={toggle}
>
  <span class="caption">Lv</span>
  <span class="value">{level}</span>
  <div class="xp-tooltip" role="tooltip">
    <strong>{xpInfo.gainedXp.toLocaleString()}</strong>
    <span> / {xpInfo.neededXp.toLocaleString()} XP</span>
    <em>({xpInfo.percent}%)</em>
  </div>
</button>

<style>
  .level-badge {
    --ring: #5ec8f0;
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    width: 50px;
    height: 50px;
    border-radius: 11px;
    color: #f0c040;
    background: rgba(20, 16, 10, 0.72);
    border: 1px solid rgba(255, 255, 255, 0.12);
    font-family: system-ui, sans-serif;
    line-height: 1;
    padding: 0;
    cursor: pointer;
    user-select: none;
    transition: box-shadow 200ms ease;
  }

  .level-badge::before {
    content: '';
    position: absolute;
    inset: -1px;
    padding: 3px;
    border-radius: inherit;
    background: conic-gradient(var(--ring) var(--xp), #7a766c 0);
    mask:
      linear-gradient(#000 0 0) content-box,
      linear-gradient(#000 0 0);
    mask-composite: exclude;
    -webkit-mask-composite: xor;
    pointer-events: none;
  }

  .level-badge.gaining {
    --ring: #a8e4ff;
    box-shadow: 0 0 8px rgba(94, 200, 240, 0.5);
  }

  .level-badge.leveling-up {
    --ring: #d6f2ff;
    box-shadow: 0 0 14px rgba(94, 200, 240, 0.7);
  }

  .level-badge:hover {
    border-color: rgba(255, 255, 255, 0.3);
  }

  .level-badge:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px rgba(159, 197, 255, 0.7);
  }

  .caption {
    color: #aaa79f;
    font-size: 7px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .value {
    margin-top: 1px;
    font-size: 28px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .xp-tooltip {
    position: absolute;
    top: calc(100% + 8px);
    left: 0;
    z-index: 1200;
    padding: 6px 9px;
    border: 1px solid rgba(216, 210, 196, 0.2);
    border-radius: 8px;
    background: rgba(12, 13, 14, 0.96);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.5);
    color: #aaa79f;
    font-size: 11px;
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    visibility: hidden;
    transform: translateY(-4px);
    transition:
      opacity 120ms ease,
      transform 120ms ease,
      visibility 120ms;
  }

  .xp-tooltip strong {
    color: #f0c040;
  }

  .xp-tooltip em {
    margin-left: 4px;
    color: #77756f;
    font-style: normal;
  }

  .level-badge:hover .xp-tooltip,
  .level-badge:focus-visible .xp-tooltip {
    opacity: 1;
    visibility: visible;
    transform: translateY(0);
  }

  @media (prefers-reduced-motion: reduce) {
    .level-badge,
    .xp-tooltip {
      transition: none;
    }
  }
</style>

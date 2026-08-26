<script lang="ts">
  import { untrack } from 'svelte'
  import { Tween } from 'svelte/motion'
  import { cubicOut } from 'svelte/easing'
  import { characterPanelVisible } from '../stores/debugStore'
  import { levelProgress } from '../utils/xpProgress'
  import {
    playerGold,
    carryWeight,
    maxCarryWeight,
    formatKg,
  } from '../stores/inventoryStore'
  import { hungerState } from '../stores/hungerStore'
  import GoldAmount from './GoldAmount.svelte'

  let {
    level,
    xp,
    hp,
    maxHp,
    str,
  }: {
    level: number
    xp: number
    hp: number
    maxHp: number
    str: number
  } = $props()

  const maxWeight = $derived(maxCarryWeight(str, $hungerState))

  const xpInfo = $derived(levelProgress(level, xp))
  const ring = new Tween(0, { duration: 300, easing: cubicOut })
  const spark = new Tween(0, { duration: 0 })
  // Flight time is `base + distance`: a launch from the gauge start gets the
  // full wind-up, a mid-flight extension only pays for the extra distance.
  const LAUNCH_MS = 420
  const EXTEND_MS = 160
  const LEVEL_HOLD_MS = 260
  const motionQuery =
    typeof matchMedia === 'function'
      ? matchMedia('(prefers-reduced-motion: reduce)')
      : null

  // Where the spark lands: the badge's rounded-rect edge, hit by the ray the
  // conic gauge draws at that progress.
  const EDGE_HALF = 24.5
  const EDGE_RADIUS = 11

  function ringPoint(progress: number) {
    const theta = progress * Math.PI * 2
    const dx = Math.sin(theta)
    const dy = -Math.cos(theta)
    let t = Math.min(
      dx === 0 ? Infinity : EDGE_HALF / Math.abs(dx),
      dy === 0 ? Infinity : EDGE_HALF / Math.abs(dy)
    )
    const flat = EDGE_HALF - EDGE_RADIUS
    if (Math.abs(dx * t) > flat && Math.abs(dy * t) > flat) {
      const cx = Math.sign(dx) * flat
      const cy = Math.sign(dy) * flat
      const b = cx * dx + cy * dy
      const c = cx * cx + cy * cy - EDGE_RADIUS * EDGE_RADIUS
      t = b + Math.sqrt(Math.max(0, b * b - c))
    }
    return { x: dx * t, y: dy * t }
  }

  let sparkOn = $state(false)
  let burst = $state(false)
  let burstAt = $state({ x: 0, y: -EDGE_HALF })
  let pulse = $state<'gain' | 'level' | null>(null)
  let pulseTimer: ReturnType<typeof setTimeout> | undefined
  let arriveTimer: ReturnType<typeof setTimeout> | undefined
  let holdTimer: ReturnType<typeof setTimeout> | undefined
  // Mid level-up: two legs, so a plain gain restarts the spark instead of
  // moving its finish line.
  let leveling = false
  let runId = 0
  let prevLevel = untrack(() => level)
  let prevXp = untrack(() => xp)

  /** Abandon whatever is in flight; every fresh start goes through here. */
  function cancel() {
    runId += 1
    clearTimeout(arriveTimer)
    clearTimeout(holdTimer)
    leveling = false
    sparkOn = false
    burst = false
  }

  function flash(kind: 'gain' | 'level') {
    pulse = kind
    clearTimeout(pulseTimer)
    pulseTimer = setTimeout(() => (pulse = null), kind === 'level' ? 1000 : 700)
  }

  // Arrival is driven by our own timer, not the tween's promise: retargeting
  // aborts the running tween, and an aborted promise never settles.
  function travel(
    to: number,
    kind: 'gain' | 'level',
    id: number,
    base: number,
    onArrive?: () => void
  ) {
    const ms = Math.round(base + Math.abs(to - spark.current) * 1000)
    clearTimeout(arriveTimer)
    clearTimeout(holdTimer)
    burst = false
    sparkOn = true
    // From wherever the head is now, so an extended flight keeps flowing.
    spark.set(to, { duration: ms, easing: cubicOut })
    arriveTimer = setTimeout(() => {
      if (id !== runId) return
      ring.set(to, { duration: 260 })
      flash(kind)
      sparkOn = false
      burstAt = ringPoint(to)
      burst = true
      onArrive?.()
    }, ms)
  }

  function launch(
    to: number,
    kind: 'gain' | 'level',
    id: number,
    onArrive?: () => void
  ) {
    spark.set(0, { duration: 0 })
    travel(to, kind, id, LAUNCH_MS, onArrive)
  }

  function play(target: number, leveled: boolean) {
    if (motionQuery?.matches) {
      cancel()
      ring.set(target)
      flash(leveled ? 'level' : 'gain')
      return
    }
    // XP that lands mid-flight moves the finish line; the spark on screen keeps
    // going and ends on the new total instead of starting over.
    if (sparkOn && !leveling && !leveled) {
      travel(target, 'gain', runId, EXTEND_MS)
      return
    }
    cancel()
    const id = runId
    if (!leveled) {
      launch(target, 'gain', id)
      return
    }
    leveling = true
    launch(1, 'level', id, () => {
      holdTimer = setTimeout(() => {
        if (id !== runId) return
        burst = false
        ring.set(0, { duration: 0 })
        launch(target, 'level', id, () => (leveling = false))
      }, LEVEL_HOLD_MS)
    })
  }

  $effect(() => {
    const target = xpInfo.progress
    const leveled = level > prevLevel
    const gained = xp > prevXp
    prevLevel = level
    prevXp = xp
    untrack(() => {
      if (leveled || gained) {
        play(target, leveled)
      } else {
        cancel()
        ring.set(target)
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
  class:sparking={sparkOn}
  style:--xp={`${ring.current * 100}%`}
  style:--spark={`${spark.current * 100}%`}
  aria-label={`Level ${level}, ${xpInfo.gainedXp} of ${xpInfo.neededXp} XP (${xpInfo.percent}%), HP ${hp} of ${maxHp}. Open character panel`}
  onclick={toggle}
>
  {#if burst}
    <span
      class="burst"
      style:--bx={`${burstAt.x}px`}
      style:--by={`${burstAt.y}px`}
      onanimationend={() => (burst = false)}
    ></span>
  {/if}
  <span class="caption">Lv</span>
  <span class="value">{level}</span>
  <div class="xp-tooltip" role="tooltip">
    <div>
      <strong>{xpInfo.gainedXp.toLocaleString()}</strong>
      <span> / {xpInfo.neededXp.toLocaleString()} XP</span>
      <em>({xpInfo.percent}%)</em>
    </div>
    <div>
      <strong class="hp">{Math.round(hp).toLocaleString()}</strong>
      <span> / {Math.round(maxHp).toLocaleString()} HP</span>
    </div>
    <div class="gold"><GoldAmount copper={$playerGold} /></div>
    <div>
      <strong class="weight">{formatKg($carryWeight)}</strong>
      <span> / {formatKg(maxWeight)} kg</span>
    </div>
  </div>
</button>

<style>
  /* Typed, so the per-frame spark position substitutes instead of re-parsing
     the gradient's token stream. */
  @property --spark {
    syntax: '<percentage>';
    inherits: true;
    initial-value: 0%;
  }

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

  .level-badge::before,
  .level-badge::after {
    content: '';
    position: absolute;
    inset: -1px;
    padding: 3px;
    border-radius: inherit;
    mask:
      linear-gradient(#000 0 0) content-box,
      linear-gradient(#000 0 0);
    mask-composite: exclude;
    -webkit-mask-composite: xor;
    pointer-events: none;
  }

  .level-badge::before {
    background: conic-gradient(var(--ring) var(--xp), #7a766c 0);
  }

  .level-badge::after {
    inset: -2px;
    padding: 4px;
    background: conic-gradient(
      transparent max(0%, calc(var(--spark, 0%) - 11%)),
      rgba(168, 228, 255, 0.6) max(0%, calc(var(--spark, 0%) - 5.5%)),
      rgba(232, 249, 255, 0.95) max(0%, calc(var(--spark, 0%) - 2%)),
      #ffffff var(--spark, 0%),
      rgba(232, 249, 255, 0.7) min(100%, calc(var(--spark, 0%) + 1%)),
      transparent min(100%, calc(var(--spark, 0%) + 2%))
    );
    opacity: 0;
    transition: opacity 260ms ease;
  }

  .level-badge.sparking::after {
    opacity: 1;
    transition-duration: 60ms;
  }

  .burst {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 26px;
    height: 26px;
    margin: -13px 0 0 -13px;
    border-radius: 50%;
    background: radial-gradient(
      circle,
      rgba(226, 246, 255, 0.9) 0%,
      rgba(120, 196, 240, 0.4) 36%,
      transparent 68%
    );
    pointer-events: none;
    animation: burst-glow 340ms ease-out forwards;
  }

  .burst::after {
    content: '';
    position: absolute;
    inset: 4px;
    background: #ffffff;
    clip-path: polygon(
      50% 0%,
      59% 41%,
      100% 50%,
      59% 59%,
      50% 100%,
      41% 59%,
      0% 50%,
      41% 41%
    );
    animation: burst-star 340ms ease-out forwards;
  }

  @keyframes burst-glow {
    0% {
      opacity: 0;
      transform: translate(var(--bx), var(--by)) scale(0.4);
    }
    30% {
      opacity: 1;
      transform: translate(var(--bx), var(--by)) scale(1.15);
    }
    100% {
      opacity: 0;
      transform: translate(var(--bx), var(--by)) scale(1.4);
    }
  }

  @keyframes burst-star {
    0% {
      opacity: 0.6;
      transform: scale(0.35) rotate(-20deg);
    }
    35% {
      opacity: 1;
      transform: scale(1.15) rotate(0deg);
    }
    100% {
      opacity: 0;
      transform: scale(0.8) rotate(14deg);
    }
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
    text-align: left;
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

  .xp-tooltip div + div {
    margin-top: 3px;
  }

  .xp-tooltip strong {
    color: #f0c040;
  }

  .xp-tooltip strong.hp {
    color: #e05a4d;
  }

  .xp-tooltip strong.weight {
    color: #6ba3d6;
  }

  .xp-tooltip .gold {
    font-weight: 700;
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

    .level-badge::after,
    .burst {
      display: none;
    }
  }
</style>

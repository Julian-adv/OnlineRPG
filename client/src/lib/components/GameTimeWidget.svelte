<script lang="ts" module>
  export const gameTimeState = $state({
    hour: 12,
    serverHour: 12,
    date: { year: 217, month: 1, day: 1 },
  })

  export const eclipseState = $state({ factor: 0 })

  export function setGameHour(displayHour: number, serverHour: number) {
    gameTimeState.hour = ((displayHour % 24) + 24) % 24
    gameTimeState.serverHour = ((serverHour % 24) + 24) % 24
  }

  export function setGameDate(year: number, month: number, day: number) {
    gameTimeState.date = {
      year: Math.max(1, Math.floor(year)),
      month: Math.min(12, Math.max(1, Math.floor(month))),
      day: Math.min(30, Math.max(1, Math.floor(day))),
    }
  }
</script>

<script lang="ts">
  import { calendarVisible } from '../stores/debugStore'
  import { currentDungeonDepth, isUnderground } from '../stores/dungeonStore'
  import { getSolarDaylightWindow } from '../utils/celestialSimulation'
  import {
    type MoonDefinition,
    ELDER_MOON_DEFINITION,
    SWIFT_MOON_DEFINITION,
    SUN_AXIAL_TILT_DEG,
    SUN_LATITUDE_DEG,
    getDayFactor,
    getGameCalendarDayIndex,
    getMoonDirection,
    getMoonPhaseLabel,
    getMoonPhaseState,
    getMoonTrackState,
    getSunDirection,
    getSunElevation,
    getSunTrackState,
    getTwilightBlendFactor,
    moonPhaseCanvasAction,
  } from '../utils/celestialSimulation'

  const SUN_LEFT_MARGIN_PERCENT = 2
  const SUN_RIGHT_MARGIN_PERCENT = 98
  const HORIZON_Y_PERCENT = 104
  const SUN_ARC_HEIGHT_PERCENT = 98
  const SUN_ARC_CLAMP_HEIGHT_PERCENT = 98
  const MOON_ARC_HEIGHT_PERCENT = 82
  const MOON_ARC_CLAMP_HEIGHT_PERCENT = 90
  const MOON_DAYLIGHT_VISIBILITY_SCALE = 0.45
  const SUN_MIN_Y_PERCENT = HORIZON_Y_PERCENT - SUN_ARC_CLAMP_HEIGHT_PERCENT
  const MOON_MIN_Y_PERCENT = HORIZON_Y_PERCENT - MOON_ARC_CLAMP_HEIGHT_PERCENT
  const ECLIPSE_NEW_MOON_THRESHOLD = 0.15
  const ECLIPSE_ANGLE_THRESHOLD_RAD = 0.2 // ~11.5°
  const ECLIPSE_X_OVERLAP_THRESHOLD_PERCENT = 15

  const MONTH_NAMES = [
    'Dawnmere',
    'Reson',
    'Verdant',
    'Highsun',
    'Emberfall',
    'Redrain',
    'Harvestwind',
    'Gloam',
    'Riftwane',
    'Mistveil',
    'Frostrest',
    'Afterglow',
  ] as const

  interface MoonVisualDefinition extends MoonDefinition {
    sizePx: number
    hueRotateDeg: number
    saturation: number
  }

  interface MoonVisualState {
    id: MoonVisualDefinition['id']
    displayName: string
    cycleDay: number
    periodDays: number
    phaseLabel: string
    illumination: number
    isWaxing: boolean
    xPercent: number
    yPercent: number
    sizePx: number
    hueRotateDeg: number
    saturation: number
    isVisible: boolean
    opacity: number
    eclipseFactor: number
  }

  const MOONS: readonly MoonVisualDefinition[] = [
    {
      ...ELDER_MOON_DEFINITION,
      sizePx: 18,
      hueRotateDeg: 0,
      saturation: 1,
    },
    {
      ...SWIFT_MOON_DEFINITION,
      sizePx: 14,
      hueRotateDeg: 12,
      saturation: 1,
    },
  ] as const

  function getMoonVisualState(
    moon: MoonVisualDefinition,
    hour: number,
    absoluteDayIndex: number,
    isDaylight: boolean,
    sunXPercent: number,
    month: number,
    day: number
  ): MoonVisualState {
    const phaseState = getMoonPhaseState(moon, absoluteDayIndex, hour)
    const phaseLabel = getMoonPhaseLabel(
      phaseState.illumination,
      phaseState.isWaxing
    )
    const trackState = getMoonTrackState({
      phaseState,
      isDaylight,
      leftPercent: SUN_LEFT_MARGIN_PERCENT,
      rightPercent: SUN_RIGHT_MARGIN_PERCENT,
      horizonYPercent: HORIZON_Y_PERCENT,
      arcHeightPercent: MOON_ARC_HEIGHT_PERCENT,
      daylightVisibilityScale: MOON_DAYLIGHT_VISIBILITY_SCALE,
    })

    const xPercent = trackState.xPercent
    const yPercent = Math.max(trackState.yPercent, MOON_MIN_Y_PERCENT)
    const isVisible = trackState.isVisible
    const visibilityScale = isDaylight ? MOON_DAYLIGHT_VISIBILITY_SCALE : 1

    // Eclipse: use 3D angular separation to account for moon's orbital inclination.
    // Eclipses only occur when new moon is near an orbital node (sun/moon declinations align).
    const phaseFactor = Math.max(
      0,
      1 - phaseState.illumination / ECLIPSE_NEW_MOON_THRESHOLD
    )
    let eclipseFactor = 0
    if (isDaylight && isVisible && phaseFactor > 0) {
      const moonDir = getMoonDirection(
        phaseState,
        SUN_LATITUDE_DEG,
        absoluteDayIndex
      )
      const sunDir = getSunDirection({ hour, month, day })
      const dot =
        moonDir.x * sunDir.x + moonDir.y * sunDir.y + moonDir.z * sunDir.z
      const angularSep = Math.acos(Math.max(-1, Math.min(1, dot)))
      const directionFactor = Math.max(
        0,
        1 - angularSep / ECLIPSE_ANGLE_THRESHOLD_RAD
      )
      eclipseFactor = directionFactor * phaseFactor
    }

    // Hide moon when visually overlapping the sun but not eclipsing.
    const xOverlapFactor =
      isDaylight && isVisible
        ? Math.max(
            0,
            1 -
              Math.abs(xPercent - sunXPercent) /
                ECLIPSE_X_OVERLAP_THRESHOLD_PERCENT
          )
        : 0
    const visibilityModifier = Math.min(
      1,
      Math.max(0, 1 - xOverlapFactor + eclipseFactor)
    )
    const eclipseBoost = eclipseFactor * (1 - visibilityScale)
    const opacity = isVisible
      ? visibilityScale * visibilityModifier + eclipseBoost
      : 0

    return {
      id: moon.id,
      displayName: moon.displayName,
      cycleDay: phaseState.cycleDay,
      periodDays: moon.periodDays,
      phaseLabel,
      illumination: phaseState.illumination,
      isWaxing: phaseState.isWaxing,
      xPercent,
      yPercent,
      sizePx: moon.sizePx,
      hueRotateDeg: moon.hueRotateDeg,
      saturation: moon.saturation,
      isVisible,
      opacity,
      eclipseFactor,
    }
  }

  function getMoonGlowFilter(
    moon: MoonVisualState,
    isDaylight: boolean
  ): string {
    if (moon.eclipseFactor > 0.05) {
      const e = moon.eclipseFactor
      const innerR = Math.round(2 + e * 5)
      const outerR = Math.round(6 + e * 12)
      const innerA = (e * 0.95).toFixed(2)
      const outerA = (e * 0.55).toFixed(2)
      return `drop-shadow(0 0 ${innerR}px rgba(255,220,100,${innerA})) drop-shadow(0 0 ${outerR}px rgba(255,150,30,${outerA}))`
    }
    return isDaylight
      ? 'drop-shadow(0 0 3px rgba(255,255,255,0.95)) drop-shadow(0 0 1px rgba(60,80,130,0.6))'
      : 'drop-shadow(0 0 4px rgba(215,228,255,0.65))'
  }

  function formatGameDate() {
    const monthName =
      MONTH_NAMES[gameTimeState.date.month - 1] ??
      `Month ${gameTimeState.date.month}`
    const day = gameTimeState.date.day.toString().padStart(2, '0')
    return `${gameTimeState.date.year} ${monthName} ${day}`
  }

  function formatGameTime() {
    const h = Math.floor(gameTimeState.hour)
    const m = Math.floor((gameTimeState.hour - h) * 60)
    return `${h.toString().padStart(2, '0')}:${m.toString().padStart(2, '0')}`
  }

  function getCurrentDaylightWindow() {
    return getSolarDaylightWindow({
      latitudeDeg: SUN_LATITUDE_DEG,
      month: gameTimeState.date.month,
      day: gameTimeState.date.day,
      axialTiltDeg: SUN_AXIAL_TILT_DEG,
    })
  }

  function getSunVisualState(
    hour: number,
    sunriseHour: number,
    sunsetHour: number
  ) {
    const trackState = getSunTrackState({
      hour,
      sunriseHour,
      sunsetHour,
      leftPercent: SUN_LEFT_MARGIN_PERCENT,
      rightPercent: SUN_RIGHT_MARGIN_PERCENT,
      horizonYPercent: HORIZON_Y_PERCENT,
      arcHeightPercent: SUN_ARC_HEIGHT_PERCENT,
    })

    return {
      ...trackState,
      yPercent: Math.max(trackState.yPercent, SUN_MIN_Y_PERCENT),
    }
  }

  const daylightWindow = $derived(getCurrentDaylightWindow())
  const sunVisual = $derived(
    getSunVisualState(
      gameTimeState.hour,
      daylightWindow.sunriseHour,
      daylightWindow.sunsetHour
    )
  )
  const sunElevation = $derived(
    getSunElevation({
      hour: gameTimeState.hour,
      month: gameTimeState.date.month,
      day: gameTimeState.date.day,
    })
  )
  const twilightBlend = $derived(getTwilightBlendFactor(sunElevation))
  const dayFactor = $derived(getDayFactor(sunElevation))
  const absoluteDayIndex = $derived(getGameCalendarDayIndex(gameTimeState.date))
  const nightSkyOffsetPx = $derived(() => {
    const { month, day } = gameTimeState.date
    const dayOfYear = (month - 1) * 30 + day // 1 to 360
    const seasonalOffset = ((dayOfYear - 1) / 360) * 512
    const rotationOffset = (gameTimeState.hour / 24) * 512
    return seasonalOffset + rotationOffset
  })
  const moonVisuals = $derived(
    $isUnderground
      ? []
      : MOONS.map((moon) =>
          getMoonVisualState(
            moon,
            gameTimeState.hour,
            absoluteDayIndex,
            sunVisual.isDaylight,
            sunVisual.xPercent,
            gameTimeState.date.month,
            gameTimeState.date.day
          )
        )
  )

  $effect(() => {
    eclipseState.factor = Math.max(
      0,
      ...moonVisuals.map((m) => m.eclipseFactor)
    )
  })
</script>

<div class="time-widget" class:compact={!$calendarVisible}>
  {#if $calendarVisible}
    <div class="meta">
      <span class="date">{formatGameDate()}</span>
      <span class="time">{formatGameTime()}</span>
    </div>
  {/if}
  <div class="sky-track">
    {#if $isUnderground}
      <img class="horizon dungeon" src="/icons/horizon-dungeon.png" alt="" />
      <span class="dungeon-floor">{$currentDungeonDepth}</span>
    {:else}
      <div
        class="night-sky"
        style="background-position-x: {nightSkyOffsetPx()}px; opacity: {1 -
          dayFactor}"
      ></div>
      <img
        class="horizon"
        src="/icons/horizon.png"
        alt=""
        style="opacity: {dayFactor * (1 - twilightBlend)}"
      />
      <img
        class="horizon"
        src="/icons/horizon-sunset.png"
        alt=""
        style="opacity: {twilightBlend}"
      />
      {#if sunVisual.isDaylight}
        <img
          class="sun"
          src="/icons/sun.png"
          alt="Sun"
          style={`--sun-x:${sunVisual.xPercent}%; --sun-y:${sunVisual.yPercent}%`}
        />
      {/if}
      {#each moonVisuals as moon (moon.id)}
        <canvas
          class="moon"
          aria-label={`${moon.displayName} Moon`}
          use:moonPhaseCanvasAction={{
            moonId: moon.id,
            illumination: moon.illumination,
            isWaxing: moon.isWaxing,
            sizePx: moon.sizePx,
            isDaylight: sunVisual.isDaylight,
          }}
          style={`--moon-x:${moon.xPercent}%; --moon-y:${moon.yPercent}%; --moon-size:${moon.sizePx}px; --moon-opacity:${moon.opacity}; --moon-hue:${moon.hueRotateDeg}deg; --moon-saturation:${moon.saturation}; --moon-glow:${getMoonGlowFilter(moon, sunVisual.isDaylight)};`}
        ></canvas>
      {/each}
      <img
        class="horizon-front"
        src="/icons/horizon-night-front.png"
        alt=""
        style="opacity: {1 - dayFactor}"
      />
      <img
        class="horizon-front"
        src="/icons/horizon-sunset-front.png"
        alt=""
        style="opacity: {twilightBlend}"
      />
      <img
        class="horizon-front"
        src="/icons/horizon-front.png"
        alt=""
        style="opacity: {dayFactor * (1 - twilightBlend)}"
      />
    {/if}
  </div>
</div>

<style>
  .time-widget {
    position: fixed;
    top: 9px;
    right: 9px;
    z-index: 1000;
    pointer-events: none;
    background: rgba(0, 0, 0, 0.8);
    color: #f7f1d0;
    border-radius: 10px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.45);
    padding: 5px;
    font-family: 'Courier New', monospace;
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .time-widget.compact {
    background: transparent;
    box-shadow: none;
    /* No background box in compact mode, so drop the padding that would
       otherwise push the visible sky-track past the 9px edge margin. */
    padding: 0;
    border-radius: 0;
    width: auto;
  }

  .compact .sky-track {
    flex: none;
    width: 256px;
  }

  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 108px;
  }

  .sky-track {
    position: relative;
    flex: 1;
    width: 256px;
    height: 36px;
    border-radius: 8px;
    overflow: hidden;
    background: linear-gradient(
      180deg,
      rgba(130, 210, 255, 0.82) 0%,
      rgba(85, 170, 230, 0.72) 55%,
      rgba(22, 43, 74, 0.5) 100%
    );
  }

  .night-sky {
    position: absolute;
    inset: 0;
    background-image: url('/icons/night-sky-panorama-512.png');
    background-size: 512px 100%;
    background-repeat: repeat-x;
    z-index: 0;
  }

  .horizon {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 100%;
    height: 100%;
    object-fit: fill;
    opacity: 0.95;
    z-index: 1;
  }

  .horizon.dungeon {
    opacity: 1;
  }

  .dungeon-floor {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    padding-left: 8px;
    z-index: 2;
    font-family: system-ui, sans-serif;
    font-size: 15px;
    font-weight: bold;
    color: #f2e2b0;
    text-shadow:
      0 0 6px rgba(0, 0, 0, 0.9),
      0 1px 2px rgba(0, 0, 0, 0.95);
  }

  .sun {
    position: absolute;
    width: 32px;
    height: 32px;
    left: var(--sun-x);
    top: var(--sun-y);
    transform: translate(-50%, -50%);
    filter: drop-shadow(0 0 6px rgba(255, 225, 100, 0.85));
    opacity: 1;
    z-index: 2;
  }

  .moon {
    position: absolute;
    width: var(--moon-size);
    height: var(--moon-size);
    left: var(--moon-x);
    top: var(--moon-y);
    transform: translate(-50%, -50%);
    opacity: var(--moon-opacity);
    filter: saturate(var(--moon-saturation)) hue-rotate(var(--moon-hue))
      var(--moon-glow);
    z-index: 2;
  }

  .horizon-front {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 100%;
    height: 100%;
    object-fit: fill;
    z-index: 3;
  }

  .date {
    font-size: 12px;
    opacity: 0.9;
    line-height: 1;
    white-space: nowrap;
    min-width: 108px;
    text-align: left;
  }

  .time {
    font-size: 14px;
    font-weight: bold;
    opacity: 0.95;
    line-height: 1;
    white-space: nowrap;
    letter-spacing: 1px;
  }

  @media (orientation: portrait) and (pointer: coarse) and (max-width: 900px) {
    .time-widget {
      top: max(10px, env(safe-area-inset-top));
    }
  }

  @media (orientation: landscape) and (pointer: coarse) and (max-height: 600px) {
    .time-widget {
      top: 2px;
      right: max(10px, env(safe-area-inset-right));
    }
  }
</style>

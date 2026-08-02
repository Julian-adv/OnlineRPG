import { writable, derived, get } from 'svelte/store'
import { refractionEnabled, reflectionEnabled } from './debugStore'
import { TARGET_FPS } from '../utils/frameTiming'
import type { GpuBenchmarkResult } from '../utils/gpuBenchmark'

export type QualityLevel = 'high' | 'medium' | 'low'
export type RenderBudget = 'full' | 'mobile'

export interface GraphicsPreset {
  renderBudget: RenderBudget
  pixelRatioCap: number
  /** Main-scene render cap. Capping at TARGET_FPS is free — the simulation
   *  produces no new state above it. Going lower trades motion for GPU time. */
  maxRenderFps: number
  shadowMapSize: number
  torchShadowMapSize: number
  antialias: boolean
  refraction: boolean
  reflection: boolean
  /** Screen-size divisor for the refraction/reflection render targets. */
  waterRtDivisor: number
  /** Allow the per-pixel depth-buffer shoreline. Costs a full-res depth copy
   *  every frame water is on screen; the heightmap-ramp fallback is free. */
  waterPixelDepth: boolean
  grassDensity: number
  enableDirectionalShadows: boolean
  enableWaterLayer: boolean
  enableWaterEffects: boolean
  enableGrassLayer: boolean
  grassCastsShadow: boolean
  enableTreeLayer: boolean
  treeInstanceLimit: number
  treeCastsShadow: boolean
  enableWindParticles: boolean
  enableHousingLayer: boolean
  enableTorchEffects: boolean
  enableTorchShadows: boolean
  terrainQueueDrainTilesBeforeStagger: number
  terrainTileWorkPerFrame: number | undefined
  terrainMaterialPrecompilePoolSize: number
  initialTerrainQueueDrainCount: number
  initialTileWorkDrainCount: number
  warmupScenePipelines: boolean
  worldMapDefaultZoomSpan: number
  worldMapMaxZoomSpan: number
  worldMapImageCacheLimit: number
}

const FULL_RENDER_SETTINGS = {
  renderBudget: 'full',
  maxRenderFps: TARGET_FPS,
  enableDirectionalShadows: true,
  enableWaterLayer: true,
  enableWaterEffects: true,
  enableGrassLayer: true,
  enableTreeLayer: true,
  enableHousingLayer: true,
  enableTorchEffects: true,
  enableTorchShadows: true,
  terrainQueueDrainTilesBeforeStagger: Infinity,
  terrainTileWorkPerFrame: undefined,
  terrainMaterialPrecompilePoolSize: 8,
  initialTerrainQueueDrainCount: Infinity,
  initialTileWorkDrainCount: Infinity,
  warmupScenePipelines: true,
  worldMapDefaultZoomSpan: 8,
  worldMapMaxZoomSpan: 32,
} satisfies Omit<
  GraphicsPreset,
  | 'pixelRatioCap'
  | 'shadowMapSize'
  | 'torchShadowMapSize'
  | 'antialias'
  | 'refraction'
  | 'reflection'
  | 'waterRtDivisor'
  | 'waterPixelDepth'
  | 'grassDensity'
  | 'grassCastsShadow'
  | 'treeInstanceLimit'
  | 'treeCastsShadow'
  | 'enableWindParticles'
  | 'worldMapImageCacheLimit'
>

const PRESETS: Record<QualityLevel, GraphicsPreset> = {
  high: {
    ...FULL_RENDER_SETTINGS,
    pixelRatioCap: 2.0,
    shadowMapSize: 4096,
    torchShadowMapSize: 1024,
    antialias: true,
    refraction: true,
    reflection: true,
    waterRtDivisor: 2,
    waterPixelDepth: true,
    grassDensity: 1.0,
    grassCastsShadow: true,
    treeInstanceLimit: 1024,
    treeCastsShadow: true,
    enableWindParticles: true,
    worldMapImageCacheLimit: Infinity,
  },
  medium: {
    ...FULL_RENDER_SETTINGS,
    pixelRatioCap: 1.5,
    shadowMapSize: 2048,
    torchShadowMapSize: 512,
    antialias: false,
    refraction: true,
    reflection: true,
    waterRtDivisor: 2,
    waterPixelDepth: true,
    grassDensity: 0.7,
    grassCastsShadow: false,
    treeInstanceLimit: 768,
    treeCastsShadow: true,
    enableWindParticles: true,
    worldMapImageCacheLimit: 256,
  },
  low: {
    ...FULL_RENDER_SETTINGS,
    pixelRatioCap: 1.0,
    shadowMapSize: 1024,
    torchShadowMapSize: 256,
    antialias: false,
    refraction: false,
    reflection: false,
    waterRtDivisor: 3,
    waterPixelDepth: false,
    grassDensity: 0.4,
    grassCastsShadow: false,
    treeInstanceLimit: 512,
    treeCastsShadow: false,
    enableWindParticles: false,
    worldMapImageCacheLimit: 128,
  },
}

const STORAGE_KEY = 'onlinerpg_graphicsQuality'
const STORAGE_KEY_APPLIED_AA = 'onlinerpg_appliedAA'
/** Used only when the GPU probe can't run (no WebGPU, timeout, failure). */
const DEFAULT_QUALITY: QualityLevel = 'medium'

// Snapshot taken at module init, before `graphicsQuality`'s subscriber
// persists its initial value — that write lands synchronously on
// subscription and would otherwise erase the "never chosen" state.
const _hadStoredQuality = readStoredQuality() !== null
/** Set once the level is settled for this session — by the probe landing, or
 *  by the player picking one while the probe was still in flight. */
let _autoQualityLocked = false

// Device render budgets are constant for the session, so compute each once and
// cache it. They're read on hot paths (grass streaming, graphics-quality changes)
// where re-running matchMedia/UA-regex per call would be wasted work.
let _mobileBudgetDetected: boolean | undefined
let _renderBudget: RenderBudget | undefined

function detectMobileRenderBudget(): boolean {
  if (_mobileBudgetDetected !== undefined) return _mobileBudgetDetected
  if (typeof window === 'undefined') return false

  const coarsePointer =
    window.matchMedia?.('(pointer: coarse)').matches ?? false
  const narrowViewport = Math.min(window.innerWidth, window.innerHeight) <= 600
  const touchDevice = navigator.maxTouchPoints > 0

  _mobileBudgetDetected = touchDevice && (coarsePointer || narrowViewport)
  return _mobileBudgetDetected
}

export function getDeviceRenderBudget(): RenderBudget {
  if (_renderBudget !== undefined) return _renderBudget
  if (typeof window === 'undefined') return 'full'

  const ua = navigator.userAgent
  const explicitIphone = /\biPhone\b/.test(ua)
  const tinyTouchViewport =
    navigator.maxTouchPoints > 0 &&
    Math.min(window.innerWidth, window.innerHeight) <= 430

  _renderBudget =
    detectMobileRenderBudget() || explicitIphone || tinyTouchViewport
      ? 'mobile'
      : 'full'
  return _renderBudget
}

function getMobileSafePreset(preset: GraphicsPreset): GraphicsPreset {
  return {
    ...preset,
    renderBudget: 'mobile',
    maxRenderFps: Math.min(preset.maxRenderFps, 30),
    pixelRatioCap: Math.min(preset.pixelRatioCap, 0.75),
    shadowMapSize: Math.min(preset.shadowMapSize, 512),
    torchShadowMapSize: 256,
    antialias: false,
    refraction: false,
    reflection: false,
    waterRtDivisor: 3,
    waterPixelDepth: false,
    grassDensity: 0,
    enableDirectionalShadows: true,
    enableWaterLayer: true,
    enableWaterEffects: false,
    enableGrassLayer: false,
    grassCastsShadow: false,
    enableTreeLayer: true,
    treeInstanceLimit: 384,
    treeCastsShadow: false,
    enableWindParticles: false,
    enableHousingLayer: true,
    enableTorchEffects: true,
    enableTorchShadows: false,
    terrainQueueDrainTilesBeforeStagger: Infinity,
    terrainTileWorkPerFrame: 1,
    terrainMaterialPrecompilePoolSize: 1,
    initialTerrainQueueDrainCount: 1,
    initialTileWorkDrainCount: 2,
    warmupScenePipelines: false,
    worldMapDefaultZoomSpan: 2,
    worldMapMaxZoomSpan: 4,
    worldMapImageCacheLimit: 32,
  }
}

function readStoredQuality(): QualityLevel | null {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored === 'high' || stored === 'medium' || stored === 'low')
      return stored
  } catch {
    // localStorage unavailable
  }
  return null
}

function loadQuality(): QualityLevel {
  return readStoredQuality() ?? DEFAULT_QUALITY
}

/** True when nothing had ever been stored for this browser, so the GPU probe
 *  should decide. A stored value — auto-picked or hand-picked — always wins;
 *  the probe never overrides a level the player is already running. */
export function needsAutoQuality(): boolean {
  return !_hadStoredQuality && !_autoQualityLocked
}

/** Map a `runGpuBenchmark` score onto a preset.
 *
 * Two measured anchors, both Chrome 150:
 *   RTX 4090 ~1730 (±8%)   -> high
 *   M3 10-core ~92 (±1.5%) -> medium
 * The medium cut sits at 70 so the M3 clears it with roughly 30% of margin.
 *
 * That the M3 lands in medium is deliberate. An M3 MacBook Air 15 ran ~30fps
 * on the medium that predates the preset trim (grass shadows off, density
 * 0.7, tree cap 768, torch shadow 512) and ~50fps after it. Low reads as too
 * stripped-down to ship as a modern laptop's default, so if the M3 ever needs
 * more headroom, make medium cheaper rather than lowering it into low.
 *
 * The high cut is pure interpolation; no mid-range discrete part has been
 * measured. It leans conservative on purpose: guessing medium for a fast
 * machine costs fidelity the player can undo in settings, while guessing high
 * for a slow one is a bad first five minutes. */
const AUTO_QUALITY_THRESHOLDS: { min: number; level: QualityLevel }[] = [
  { min: 600, level: 'high' },
  { min: 70, level: 'medium' },
]

export function qualityForScore(score: number): QualityLevel {
  for (const { min, level } of AUTO_QUALITY_THRESHOLDS) {
    if (score >= min) return level
  }
  return 'low'
}

/** Map a conclusive probe outcome onto a preset. No WebGPU adapter means the
 *  renderer falls back to WebGL — often software rendering — so it lands on
 *  the slowest tier regardless of the actual hardware. */
export function qualityForOutcome(
  outcome: GpuBenchmarkResult | 'no-webgpu'
): QualityLevel {
  return outcome === 'no-webgpu' ? 'low' : qualityForScore(outcome.score)
}

/** Persist a probe-derived level. No-op once anything is stored. */
export function applyAutoQuality(level: QualityLevel): void {
  if (!needsAutoQuality()) return
  _autoQualityLocked = true
  graphicsQuality.set(level)
}

/** An explicit choice from the settings panel. Locks out the probe, which on
 *  a first launch may still be in flight and would otherwise overwrite it. */
export function setQualityManual(level: QualityLevel): void {
  _autoQualityLocked = true
  graphicsQuality.set(level)
}

/**
 * Called once at renderer creation time.
 * Returns the antialias flag and records what was applied
 * so `reloadNeeded` can detect mismatches later.
 */
export function applyInitialAntialias(): boolean {
  const aa = getEffectivePreset(loadQuality()).antialias
  try {
    localStorage.setItem(STORAGE_KEY_APPLIED_AA, String(aa))
  } catch {
    // localStorage unavailable
  }
  return aa
}

/** Antialias flag the renderer was actually created with this session.
 *  Falls back to the current preset when the applied record is missing
 *  (first launch — `applyInitialAntialias` hasn't stored it yet). */
export function getAppliedAntialias(): boolean {
  try {
    const stored = localStorage.getItem(STORAGE_KEY_APPLIED_AA)
    if (stored !== null) return stored === 'true'
  } catch {
    // localStorage unavailable
  }
  return getEffectivePreset(loadQuality()).antialias
}

export const graphicsQuality = writable<QualityLevel>(loadQuality())

// Device budget and base presets are constant for the session, so the effective
// preset is a pure function of `level`. Cache it — `getCurrentPreset()` is read
// on hot paths (grass streaming) where re-spreading the mobile-safe object per
// call would be wasted allocation.
const _effectivePresetCache: Partial<Record<QualityLevel, GraphicsPreset>> = {}

/** True when the current preset's antialias differs from what the renderer was created with. */
export const reloadNeeded = derived(graphicsQuality, (level) => {
  try {
    const appliedAA = localStorage.getItem(STORAGE_KEY_APPLIED_AA) === 'true'
    return getEffectivePreset(level).antialias !== appliedAA
  } catch {
    return false
  }
})

// Sync to localStorage and debugStore on change
graphicsQuality.subscribe((level) => {
  try {
    localStorage.setItem(STORAGE_KEY, level)
  } catch {
    // localStorage unavailable
  }
  const preset = getEffectivePreset(level)
  refractionEnabled.set(preset.refraction)
  reflectionEnabled.set(preset.reflection)
})

export function getEffectivePreset(level: QualityLevel): GraphicsPreset {
  return (_effectivePresetCache[level] ??= withSafeRenderCap(
    getDeviceRenderBudget() === 'mobile'
      ? getMobileSafePreset(PRESETS[level])
      : PRESETS[level]
  ))
}

/** A cap below 1 stops the canvas drawing at all, with nothing to show for it
 *  but a black screen — reject it here rather than frames deep. */
function withSafeRenderCap(preset: GraphicsPreset): GraphicsPreset {
  return preset.maxRenderFps >= 1
    ? preset
    : { ...preset, maxRenderFps: TARGET_FPS }
}

export function getCurrentPreset(): GraphicsPreset {
  return getEffectivePreset(get(graphicsQuality))
}

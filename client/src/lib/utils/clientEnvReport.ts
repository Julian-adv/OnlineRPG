import { get } from 'svelte/store'
import type { WebGPURenderer } from 'three/webgpu'
import {
  graphicsQuality,
  getEffectivePreset,
  getAppliedAntialias,
  type QualityLevel,
  type RenderBudget,
} from '../stores/graphicsSettings'

/** Rendering environment of one web client, reported so performance
 *  complaints can be matched against actual hardware. Sent only when the
 *  stable parts of the environment change (see `fingerprint`); the first
 *  session on a browser counts as a change. Field names are the wire shape —
 *  keep them in sync with the server struct. */
export interface ClientEnvReport {
  quality: QualityLevel
  render_budget: RenderBudget
  antialias: boolean
  /** Pixel ratio actually handed to the renderer (device ratio, preset-capped). */
  pixel_ratio: number
  device_pixel_ratio: number
  viewport_w: number
  viewport_h: number
  screen_w: number
  screen_h: number
  /** 'webgpu' | 'webgl' — three falls back when WebGPU is unavailable. */
  backend: string
  gpu_vendor: string
  gpu_architecture: string
  gpu_device: string
  gpu_description: string
  user_agent: string
}

interface AdapterInfoLike {
  vendor?: string
  architecture?: string
  device?: string
  description?: string
}

/** Bound every string so a hostile or merely verbose client can't write
 *  unbounded log lines. */
function clamp(value: string | undefined, max: number): string {
  return (value ?? '').slice(0, max)
}

/** three 0.185's WebGPUBackend keeps the device but not the adapter, so read
 *  `GPUDevice.adapterInfo` and fall back to requesting a throwaway adapter.
 *  Browsers deliberately coarsen these values (vendor/architecture are
 *  populated; device/description are often empty). */
async function readAdapterInfo(
  renderer: WebGPURenderer | null
): Promise<AdapterInfoLike> {
  const backend = (
    renderer as unknown as {
      backend?: { device?: { adapterInfo?: AdapterInfoLike } }
    } | null
  )?.backend
  if (backend?.device?.adapterInfo) return backend.device.adapterInfo

  const gpu = (
    navigator as Navigator & {
      gpu?: { requestAdapter(): Promise<{ info?: AdapterInfoLike } | null> }
    }
  ).gpu
  if (!gpu) return {}
  try {
    return (await gpu.requestAdapter())?.info ?? {}
  } catch {
    return {}
  }
}

function detectBackend(renderer: WebGPURenderer | null): string {
  const backend = (
    renderer as unknown as {
      backend?: { isWebGPUBackend?: boolean }
    } | null
  )?.backend
  if (!backend) return 'unknown'
  return backend.isWebGPUBackend ? 'webgpu' : 'webgl'
}

const STORAGE_KEY = 'envReportLastSent'

/** Stable parts of the report. Viewport/pixel-ratio fields are excluded so a
 *  window resize or zoom change doesn't count as a new environment. */
function fingerprint(report: ClientEnvReport): string {
  return [
    report.quality,
    report.render_budget,
    report.antialias,
    report.backend,
    report.gpu_vendor,
    report.gpu_architecture,
    report.gpu_device,
    report.gpu_description,
    report.user_agent,
  ].join('|')
}

/** Collect the report and hand it to `send` when the stable environment
 *  changed since the last successful send. Only a send that actually went
 *  out (`send` returned true) is recorded. */
export async function maybeSendEnvReport(
  renderer: WebGPURenderer | null,
  send: (report: ClientEnvReport) => boolean
): Promise<void> {
  const report = await collectClientEnvReport(renderer)
  const fp = fingerprint(report)
  try {
    if (localStorage.getItem(STORAGE_KEY) === fp) return
  } catch {
    // localStorage unavailable — report every session instead
  }
  if (!send(report)) return
  try {
    localStorage.setItem(STORAGE_KEY, fp)
  } catch {
    // ignore; next session just reports again
  }
}

export async function collectClientEnvReport(
  renderer: WebGPURenderer | null
): Promise<ClientEnvReport> {
  const quality = get(graphicsQuality)
  const preset = getEffectivePreset(quality)
  const info = await readAdapterInfo(renderer)

  return {
    quality,
    render_budget: preset.renderBudget,
    antialias: getAppliedAntialias(),
    pixel_ratio: Math.min(window.devicePixelRatio, preset.pixelRatioCap),
    device_pixel_ratio: window.devicePixelRatio,
    viewport_w: window.innerWidth,
    viewport_h: window.innerHeight,
    screen_w: window.screen?.width ?? 0,
    screen_h: window.screen?.height ?? 0,
    backend: detectBackend(renderer),
    gpu_vendor: clamp(info.vendor, 64),
    gpu_architecture: clamp(info.architecture, 64),
    gpu_device: clamp(info.device, 64),
    gpu_description: clamp(info.description, 128),
    user_agent: clamp(navigator.userAgent, 256),
  }
}

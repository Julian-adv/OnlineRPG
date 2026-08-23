import * as THREE from 'three'
import { WebGPURenderer } from 'three/webgpu'
import {
  applyInitialAntialias,
  getAppliedAntialias,
  getCurrentPreset,
} from '../stores/graphicsSettings'

// Patch Material.dispose to catch WebGPU backend race conditions.
// When Threlte disposes materials before the WebGPU backend finishes
// async init, internal Nodes.delete() crashes on undefined nodeData.
// This is a known Three.js WebGPU issue — safe to swallow.
const _origMaterialDispose = THREE.Material.prototype.dispose
THREE.Material.prototype.dispose = function () {
  try {
    _origMaterialDispose.call(this)
  } catch {
    // WebGPU backend not ready — ignore
  }
}

export function createWebGPURenderer(canvas: HTMLCanvasElement) {
  const preset = getCurrentPreset()
  const antialias = applyInitialAntialias()
  const renderer = new WebGPURenderer({ canvas, antialias })
  renderer.setPixelRatio(
    Math.min(window.devicePixelRatio, preset.pixelRatioCap)
  )
  renderer.shadowMap.type = THREE.PCFSoftShadowMap

  // Guard renderer.dispose() — Threlte calls it on Canvas unmount,
  // but WebGPU backend.info may not exist if init() hasn't completed.
  const _origDispose = renderer.dispose.bind(renderer)
  renderer.dispose = () => {
    try {
      _origDispose()
    } catch {
      // backend not yet initialized — safe to ignore
    }
  }

  return renderer
}

/** Renderer for the small secondary preview canvases. Reuses the main
 *  renderer's applied antialias without re-recording it (that record is
 *  the main canvas's restart-notice source), and defers a dispose that
 *  races init() so the GPU device is actually released. The pixel-ratio
 *  cap must come from the Canvas `dpr` prop — Threlte overwrites anything
 *  set here. */
export function createPreviewWebGPURenderer(canvas: HTMLCanvasElement) {
  const renderer = new WebGPURenderer({
    canvas,
    antialias: getAppliedAntialias(),
  })

  const _origDispose = renderer.dispose.bind(renderer)
  renderer.dispose = () => {
    renderer
      .init()
      .then(() => _origDispose())
      .catch(() => {})
  }

  return renderer
}

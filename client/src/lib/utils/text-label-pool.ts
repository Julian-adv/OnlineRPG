import * as THREE from 'three'

export interface LabelCanvas {
  canvas: HTMLCanvasElement
  ctx: CanvasRenderingContext2D
  texture: THREE.CanvasTexture
}

/** Unit quad shared by every label; the mesh scales it to the text size. */
export const labelPlane = new THREE.PlaneGeometry(1, 1)

/** Shared context for measuring text; callers set `font` before use. */
export const measureCtx = document.createElement('canvas').getContext('2d')!

export function createTextTexture(canvas: HTMLCanvasElement) {
  const texture = new THREE.CanvasTexture(canvas)
  texture.colorSpace = THREE.SRGBColorSpace
  texture.minFilter = THREE.LinearFilter
  texture.magFilter = THREE.LinearFilter
  return texture
}

const MIN_SIZE = 32
const MAX_POOLED_PER_BUCKET = 32

/** Label canvases keyed by power-of-two bucket; textures are never disposed
 *  (WebGPU sampler bindings), so released ones are reused instead. */
const buckets = new Map<string, LabelCanvas[]>()

const bucketSize = (px: number) =>
  Math.max(MIN_SIZE, THREE.MathUtils.ceilPowerOfTwo(px))
const bucketKey = (c: HTMLCanvasElement) => `${c.width}x${c.height}`

function acquire(bw: number, bh: number): LabelCanvas {
  const pooled = buckets.get(`${bw}x${bh}`)?.pop()
  if (pooled) return pooled
  const canvas = document.createElement('canvas')
  canvas.width = bw
  canvas.height = bh
  return {
    canvas,
    ctx: canvas.getContext('2d')!,
    texture: createTextTexture(canvas),
  }
}

export function releaseLabelCanvas(lc: LabelCanvas) {
  const key = bucketKey(lc.canvas)
  let list = buckets.get(key)
  if (!list) buckets.set(key, (list = []))
  if (list.length < MAX_POOLED_PER_BUCKET) list.push(lc)
}

/** Returns a canvas whose bucket fits cw×ch (reusing `prev` when it still
 *  fits) with the texture's repeat/offset set to the top-left cw×ch sub-rect. */
export function fitLabelCanvas(
  prev: LabelCanvas | null,
  cw: number,
  ch: number
): LabelCanvas {
  const bw = bucketSize(cw)
  const bh = bucketSize(ch)
  let lc = prev
  if (!lc || lc.canvas.width !== bw || lc.canvas.height !== bh) {
    if (lc) releaseLabelCanvas(lc)
    lc = acquire(bw, bh)
  }
  lc.texture.repeat.set(cw / bw, ch / bh)
  lc.texture.offset.set(0, 1 - ch / bh)
  return lc
}

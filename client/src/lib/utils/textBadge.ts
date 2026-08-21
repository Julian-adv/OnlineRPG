import * as THREE from 'three'

/** Outlined text drawn on a canvas cut to fit the glyphs, so it stays sharp
 *  once scaled down to world size. `pixelsPerUnit` sets that scale: the badge
 *  is `fontPx / pixelsPerUnit` world units per em. */
export interface BadgeStyle {
  /** Cache key prefix — distinct per style. */
  id: string
  fontPx: number
  pixelsPerUnit: number
  bold: boolean
  color: string
  outlineColor: string
  outlineWidth: number
}

export interface TextBadge {
  texture: THREE.CanvasTexture
  width: number
  height: number
}

/** Cache mirroring iconTextureCache: badge texts are a finite set (item names,
 *  stack sizes), so each gets one rasterize + GPU upload for the process
 *  lifetime. Cached textures are shared — never dispose them. */
const cache = new Map<string, TextBadge>()

export function makeTextBadge(text: string, style: BadgeStyle): TextBadge {
  const key = `${style.id}|${text}`
  const cached = cache.get(key)
  if (cached) return cached

  const font = `${style.bold ? 'bold ' : ''}${style.fontPx}px sans-serif`
  const pad = Math.ceil(style.outlineWidth) + 4
  const c = document.createElement('canvas')
  const ctx = c.getContext('2d')!
  ctx.font = font
  c.width = Math.ceil(ctx.measureText(text).width) + pad * 2
  c.height = Math.ceil(style.fontPx * 1.25) + pad * 2
  // Resizing the canvas reset the context state, font included.
  ctx.font = font
  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  ctx.lineJoin = 'round'
  ctx.lineCap = 'round'
  ctx.strokeStyle = style.outlineColor
  ctx.lineWidth = style.outlineWidth
  ctx.strokeText(text, c.width / 2, c.height / 2)
  ctx.fillStyle = style.color
  ctx.fillText(text, c.width / 2, c.height / 2)

  const texture = new THREE.CanvasTexture(c)
  texture.minFilter = THREE.LinearFilter
  texture.magFilter = THREE.LinearFilter
  texture.colorSpace = THREE.SRGBColorSpace
  const badge = {
    texture,
    width: c.width / style.pixelsPerUnit,
    height: c.height / style.pixelsPerUnit,
  }
  cache.set(key, badge)
  return badge
}

/** Hover name label style, shared by ground items and interactable props.
 *  Matches the player nametag (TextLabel at fontSize 0.3, outlineWidth 7),
 *  a notch smaller at 0.22 world units per em. */
export const NAME_BADGE_STYLE: BadgeStyle = {
  id: 'name',
  fontPx: 64,
  pixelsPerUnit: 288,
  bold: false,
  color: '#ffffff',
  outlineColor: '#000000',
  outlineWidth: 6,
}

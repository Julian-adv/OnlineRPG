import * as THREE from 'three'
import { MeshBasicNodeMaterial } from 'three/webgpu'
import { getHousingMaterial, HOUSING_TEXTURES } from './housing-textures'
import { WOOD_TEXTURE_IDX } from './house-geo-utils'

/** Metres of board covered by one repeat of the wood texture (tunable). */
const BOARD_TEX_TILE = 0.6

// Shared board templates with editable text on the +Z face.

export interface ShopSignParams {
  shape: 'arch' | 'plaque' | 'oval'
  texture: string
  /** Horizontal chord of the arch centreline (m). */
  width: number
  /** Radial band width — how "tall" the arch strip itself is (m). */
  height: number
  /** Front-to-back plank depth (m). */
  thickness: number
  /** How much the arch centre rises above its ends (sagitta of the centreline,
   *  m). Larger = a taller, rounder arch; smaller = a flatter arch. */
  rise: number
  /** Angular tessellation of the arch. */
  segments: number
}

export const SHOP_SIGN_DEFAULTS: ShopSignParams = {
  shape: 'arch',
  texture: 'housing/wood_shutter_1k',
  width: 3,
  height: 0.5,
  thickness: 0.1,
  rise: 0.4,
  segments: 32,
}

export interface ShopSignTextParams {
  /** Fraction of the radial band height the text occupies (0..1). */
  heightFrac: number
  /** Fraction of the arch's angular span the text spans (inset from the ends). */
  widthFrac: number
  fillColor: string
  outlineColor: string
  /** Outline width in canvas pixels. */
  outlineWidth: number
}

export const SHOP_SIGN_TEXT_DEFAULTS: ShopSignTextParams = {
  heightFrac: 0.6,
  widthFrac: 0.98,
  fillColor: '#f6e7c1',
  outlineColor: '#2c1a0c',
  outlineWidth: 12,
}

interface ShopSignStyle {
  board: Partial<ShopSignParams>
  text: Partial<ShopSignTextParams>
}

export const SHOP_SIGN_STYLES = {
  classic: { board: {}, text: {} },
  plank: {
    board: {
      shape: 'plaque',
      texture: 'housing/wood_planks_1k',
      height: 0.72,
      thickness: 0.12,
    },
    text: { widthFrac: 0.84, fillColor: '#fff2d2' },
  },
  oval: {
    board: {
      shape: 'oval',
      texture: 'housing/dark_wooden_planks_1k',
      width: 2.8,
      height: 1,
      thickness: 0.14,
    },
    text: { widthFrac: 0.8, heightFrac: 0.5, fillColor: '#f5d48b' },
  },
  weathered: {
    board: {
      texture: 'housing/weathered_planks_1k',
      height: 0.58,
      rise: 0.16,
      thickness: 0.12,
    },
    text: { widthFrac: 0.92, fillColor: '#fff5df', outlineColor: '#38352e' },
  },
} satisfies Record<string, ShopSignStyle>

export type ShopSignStyleId = keyof typeof SHOP_SIGN_STYLES

export function getShopSignStyle(
  style: ShopSignStyleId = 'classic'
): ShopSignStyle {
  return SHOP_SIGN_STYLES[style] ?? SHOP_SIGN_STYLES.classic
}

interface Arch {
  /** Inner / centreline / outer radii. */
  ri: number
  rc: number
  ro: number
  /** Half of the angular span (radians), measured from the top (vertical). */
  phi: number
  /** Y offset that recentres the bounding box on the local origin. */
  yOff: number
}

/** Derive the arch radii and angular span from the tunable parameters. */
function computeArch(p: ShopSignParams): Arch {
  // Circle through the two ends and the raised centre of the centreline.
  const rc = p.rise / 2 + (p.width * p.width) / (8 * p.rise)
  const phi = Math.asin(Math.min(1, p.width / 2 / rc)) // half angular span
  const ro = rc + p.height / 2
  const ri = rc - p.height / 2
  const maxY = ro
  const minY = ri * Math.cos(phi)
  const yOff = -(maxY + minY) / 2
  return { ri, rc, ro, phi, yOff }
}

// Extruded arch with outward-facing normals and winding.
function buildBoardGeometry(p: ShopSignParams): THREE.BufferGeometry {
  if (p.shape !== 'arch') return buildPlaqueGeometry(p)
  const { ri, rc, ro, phi, yOff } = computeArch(p)
  const hz = p.thickness / 2

  const pos: number[] = []
  const nor: number[] = []
  const uvs: number[] = []

  // World-scale UVs so the wood texture tiles at a natural size (RepeatWrapping).
  // u runs along the arch (centreline arc length), v across the band / depth.
  const uArc = (a: number): number => (rc * (a + phi)) / BOARD_TEX_TILE

  const quad = (
    a: number[],
    b: number[],
    c: number[],
    d: number[],
    n: number[],
    uvA: number[],
    uvB: number[],
    uvC: number[],
    uvD: number[]
  ) => {
    // Match winding to the outward normal for correct lighting.
    const abx = b[0] - a[0]
    const aby = b[1] - a[1]
    const abz = b[2] - a[2]
    const acx = c[0] - a[0]
    const acy = c[1] - a[1]
    const acz = c[2] - a[2]
    const gx = aby * acz - abz * acy
    const gy = abz * acx - abx * acz
    const gz = abx * acy - aby * acx
    const outward = gx * n[0] + gy * n[1] + gz * n[2] >= 0
    const emit = (v: number[], t: number[]) => {
      pos.push(v[0], v[1], v[2])
      nor.push(n[0], n[1], n[2])
      uvs.push(t[0], t[1])
    }
    const order = outward
      ? [
          [a, uvA],
          [b, uvB],
          [c, uvC],
          [a, uvA],
          [c, uvC],
          [d, uvD],
        ]
      : [
          [a, uvA],
          [c, uvC],
          [b, uvB],
          [a, uvA],
          [d, uvD],
          [c, uvC],
        ]
    for (const [v, t] of order) emit(v, t)
  }

  // Points on the front (z=+hz) / back (z=-hz) faces at radius r, angle a.
  const pf = (r: number, a: number): number[] => [
    r * Math.sin(a),
    r * Math.cos(a) + yOff,
    hz,
  ]
  const pb = (r: number, a: number): number[] => [
    r * Math.sin(a),
    r * Math.cos(a) + yOff,
    -hz,
  ]
  // Face UV at radius r, angle a: u along arch, v across the radial band.
  const uvFace = (r: number, a: number): number[] => [
    uArc(a),
    (r - ri) / BOARD_TEX_TILE,
  ]
  const depthV = p.thickness / BOARD_TEX_TILE

  const N = p.segments
  for (let i = 0; i < N; i++) {
    const a0 = -phi + (2 * phi * i) / N
    const a1 = -phi + (2 * phi * (i + 1)) / N
    const am = (a0 + a1) / 2
    const radial = [Math.sin(am), Math.cos(am), 0] // outward radial

    // Front face (+Z)
    quad(
      pf(ri, a0),
      pf(ro, a0),
      pf(ro, a1),
      pf(ri, a1),
      [0, 0, 1],
      uvFace(ri, a0),
      uvFace(ro, a0),
      uvFace(ro, a1),
      uvFace(ri, a1)
    )
    // Back face (−Z)
    quad(
      pb(ri, a1),
      pb(ro, a1),
      pb(ro, a0),
      pb(ri, a0),
      [0, 0, -1],
      uvFace(ri, a1),
      uvFace(ro, a1),
      uvFace(ro, a0),
      uvFace(ri, a0)
    )
    // Outer edge (v = depth across the plank thickness)
    quad(
      pf(ro, a0),
      pb(ro, a0),
      pb(ro, a1),
      pf(ro, a1),
      radial,
      [uArc(a0), 0],
      [uArc(a0), depthV],
      [uArc(a1), depthV],
      [uArc(a1), 0]
    )
    // Inner edge
    quad(
      pf(ri, a1),
      pb(ri, a1),
      pb(ri, a0),
      pf(ri, a0),
      [-radial[0], -radial[1], 0],
      [uArc(a1), 0],
      [uArc(a1), depthV],
      [uArc(a0), depthV],
      [uArc(a0), 0]
    )
  }

  // End caps (tangent-facing).
  const capL = [-Math.cos(-phi), Math.sin(-phi), 0]
  quad(
    pf(ri, -phi),
    pf(ro, -phi),
    pb(ro, -phi),
    pb(ri, -phi),
    capL,
    [0, 0],
    [(ro - ri) / BOARD_TEX_TILE, 0],
    [(ro - ri) / BOARD_TEX_TILE, depthV],
    [0, depthV]
  )
  const capR = [Math.cos(phi), -Math.sin(phi), 0]
  quad(
    pf(ro, phi),
    pf(ri, phi),
    pb(ri, phi),
    pb(ro, phi),
    capR,
    [0, 0],
    [(ro - ri) / BOARD_TEX_TILE, 0],
    [(ro - ri) / BOARD_TEX_TILE, depthV],
    [0, depthV]
  )

  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3))
  geo.setAttribute('normal', new THREE.Float32BufferAttribute(nor, 3))
  geo.setAttribute('uv', new THREE.Float32BufferAttribute(uvs, 2))
  geo.computeBoundingBox()
  geo.computeBoundingSphere()
  return geo
}

function buildPlaqueGeometry(p: ShopSignParams): THREE.BufferGeometry {
  const shape = new THREE.Shape()
  const w = p.width / 2
  const h = p.height / 2
  if (p.shape === 'oval') {
    shape.absellipse(0, 0, w, h, 0, Math.PI * 2, false, 0)
  } else {
    const cut = p.height * 0.2
    shape.moveTo(-w + cut, -h)
    shape.lineTo(w - cut, -h)
    shape.lineTo(w, -h + cut)
    shape.lineTo(w, h - cut)
    shape.lineTo(w - cut, h)
    shape.lineTo(-w + cut, h)
    shape.lineTo(-w, h - cut)
    shape.lineTo(-w, -h + cut)
    shape.closePath()
  }
  const geo = new THREE.ExtrudeGeometry(shape, {
    depth: p.thickness,
    bevelEnabled: false,
    steps: 1,
    curveSegments: p.segments,
  })
  geo.translate(0, 0, -p.thickness / 2)
  const uv = geo.getAttribute('uv')
  for (let i = 0; i < uv.count; i++) {
    uv.setXY(i, uv.getX(i) / BOARD_TEX_TILE, uv.getY(i) / BOARD_TEX_TILE)
  }
  geo.computeBoundingBox()
  geo.computeBoundingSphere()
  return geo
}

/** Build the shared, text-less board template (a group with one mesh). */
export function buildShopSignBoard(
  params: Partial<ShopSignParams> = {}
): THREE.Group {
  const p = { ...SHOP_SIGN_DEFAULTS, ...params }
  const geo = buildBoardGeometry(p)
  const textureIndex = HOUSING_TEXTURES.findIndex(
    (entry) => entry.glb === p.texture
  )
  const mat = getHousingMaterial(
    textureIndex < 0 ? WOOD_TEXTURE_IDX : textureIndex
  )
  const mesh = new THREE.Mesh(geo, mat)
  mesh.castShadow = true
  mesh.receiveShadow = true
  // Shared materials must survive placement removal.
  mesh.userData.isSignBoard = true
  const group = new THREE.Group()
  group.name = 'shop-sign-board'
  group.add(mesh)
  return group
}

const CANVAS_PX_PER_M = 256

/** Render the shop name as straight, centred, auto-fitted text on a canvas. */
function renderNameCanvas(
  text: string,
  arcLen: number,
  bandH: number,
  tp: ShopSignTextParams
): HTMLCanvasElement {
  const cw = Math.max(2, Math.round(arcLen * CANVAS_PX_PER_M))
  const chpx = Math.max(2, Math.round(bandH * CANVAS_PX_PER_M))
  const canvas = document.createElement('canvas')
  canvas.width = cw
  canvas.height = chpx
  const ctx = canvas.getContext('2d')!

  // Fit both axes, reserving space for the outline.
  const pad = tp.outlineWidth
  const maxW = Math.max(1, cw - pad)
  const maxH = Math.max(8, chpx - pad)
  let fontPx = maxH
  const setFont = () =>
    (ctx.font = `bold ${fontPx}px Georgia, "Times New Roman", serif`)
  setFont()
  const measured = ctx.measureText(text).width
  if (measured > 0) {
    // Font size at which the text width would exactly equal maxW.
    const widthFit = (fontPx * maxW) / measured
    fontPx = Math.max(8, Math.floor(Math.min(maxH, widthFit)))
    setFont()
  }

  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  ctx.lineJoin = 'round'
  ctx.lineCap = 'round'
  if (tp.outlineWidth > 0) {
    ctx.strokeStyle = tp.outlineColor
    ctx.lineWidth = tp.outlineWidth
    ctx.strokeText(text, cw / 2, chpx / 2)
  }
  ctx.fillStyle = tp.fillColor
  ctx.fillText(text, cw / 2, chpx / 2)
  return canvas
}

// Cache text meshes: disposing their canvas materials breaks WebGPU samplers.
export function buildShopSignText(
  text: string,
  boardParams: Partial<ShopSignParams> = {},
  textParams: Partial<ShopSignTextParams> = {}
): THREE.Mesh {
  const bp = { ...SHOP_SIGN_DEFAULTS, ...boardParams }
  const tp = { ...SHOP_SIGN_TEXT_DEFAULTS, ...textParams }
  const curved = bp.shape === 'arch'
  const { rc, phi, yOff } = curved
    ? computeArch(bp)
    : { rc: 0, phi: 0, yOff: 0 }

  const phiT = phi * tp.widthFrac
  const bandH = bp.height * tp.heightFrac
  const rti = rc - bandH / 2 // inner (bottom of text)
  const rto = rc + bandH / 2 // outer (top of text)
  const z = bp.thickness / 2 + 0.012 // 12mm proud of the wood to avoid z-fight

  // --- Canvas texture (straight text; the ribbon supplies the arch) ---
  const arcLen = curved ? rc * 2 * phiT : bp.width * tp.widthFrac
  const canvas = renderNameCanvas(text, arcLen, bandH, tp)
  const texture = new THREE.CanvasTexture(canvas)
  texture.minFilter = THREE.LinearFilter
  texture.magFilter = THREE.LinearFilter
  texture.anisotropy = 4

  const mat = new MeshBasicNodeMaterial()
  mat.map = texture
  mat.transparent = true
  mat.depthWrite = false
  mat.side = THREE.DoubleSide

  const N = curved ? Math.max(8, Math.round(bp.segments * tp.widthFrac)) : 1
  const pos: number[] = []
  const uv: number[] = []
  const nor: number[] = []
  const idx: number[] = []
  for (let i = 0; i <= N; i++) {
    const u = i / N
    const a = -phiT + 2 * phiT * u
    const s = Math.sin(a)
    const c = Math.cos(a)
    pos.push(
      curved ? rti * s : (u - 0.5) * arcLen,
      curved ? rti * c + yOff : -bandH / 2,
      z
    )
    uv.push(u, 0)
    nor.push(0, 0, 1)
    pos.push(
      curved ? rto * s : (u - 0.5) * arcLen,
      curved ? rto * c + yOff : bandH / 2,
      z
    )
    uv.push(u, 1)
    nor.push(0, 0, 1)
  }
  for (let i = 0; i < N; i++) {
    const a = i * 2
    const b = i * 2 + 1
    const cc = i * 2 + 2
    const d = i * 2 + 3
    idx.push(a, cc, d, a, d, b)
  }
  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3))
  geo.setAttribute('uv', new THREE.Float32BufferAttribute(uv, 2))
  geo.setAttribute('normal', new THREE.Float32BufferAttribute(nor, 3))
  geo.setIndex(idx)
  geo.computeBoundingBox()
  geo.computeBoundingSphere()

  const mesh = new THREE.Mesh(geo, mat)
  mesh.renderOrder = 1
  mesh.userData.isSignText = true
  return mesh
}

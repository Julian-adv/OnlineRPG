import * as THREE from 'three'
import { MeshBasicNodeMaterial } from 'three/webgpu'
import { attribute, texture, vec3 } from 'three/tsl'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type N = any

const FOOTPRINT_OPACITY_ATTR = 'aFootprintOpacity'

/** Pool size, shared by every walker on screen — one ring, one draw call.
 *  A sprinter lays ~6 prints/s, so this holds a couple of full-length trails
 *  before the ring starts eating the oldest. */
const MAX_FOOTPRINTS = 1024
/** Print size in metres (length along the walking direction). */
const PRINT_LENGTH_M = 0.34
const PRINT_WIDTH_M = 0.16
/** Distance walked between prints. */
export const STRIDE_M = 0.75
/** Lateral offset of each foot from the walk line. */
const FOOT_OFFSET_M = 0.11
/** How long a print takes to dry out. */
const PRINT_LIFETIME_S = 30
/** Fraction of that spent at full strength before it starts evaporating. */
const PRINT_HOLD = 0.5
/** Hover above the feet so the quad wins the depth test on flat ground. */
const GROUND_LIFT_M = 0.03

interface Footprint {
  age: number
  /** Opacity at birth — a nearly-dry player leaves fainter prints. */
  strength: number
}

/** Stylized wet boot print (sole + heel), one canvas texture shared by all. */
let sharedPrintTexture: THREE.Texture | null = null

function printTexture(): THREE.Texture {
  if (sharedPrintTexture) return sharedPrintTexture
  const w = 64
  const h = 128
  const canvas = document.createElement('canvas')
  canvas.width = w
  canvas.height = h
  const ctx = canvas.getContext('2d')!

  ctx.fillStyle = 'rgba(255, 255, 255, 1)'
  const blob = (cx: number, cy: number, rx: number, ry: number) => {
    ctx.beginPath()
    ctx.ellipse(cx, cy, rx, ry, 0, 0, Math.PI * 2)
    ctx.fill()
  }
  // +V is forward, so the sole sits at the top and the heel at the bottom.
  blob(w / 2, h * 0.3, w * 0.36, h * 0.19)
  blob(w / 2, h * 0.74, w * 0.27, h * 0.14)

  const tex = new THREE.CanvasTexture(canvas)
  tex.needsUpdate = true
  sharedPrintTexture = tex
  return tex
}

/** How dark the ground goes under a full-strength print (multiply factor). */
const WET_TINT: [number, number, number] = [0.36, 0.4, 0.44]

/**
 * Multiply blending, not alpha: a wet patch darkens whatever ground it is on.
 * An unlit constant colour blended normally reads as a pale smear once night
 * darkens the terrain under it — multiplying tracks the lighting for free.
 *
 * WebGPU only offers `MultiplyBlending` in its premultiplied form
 * (`dst·src + dst·(1−a)`), and refuses it outright without
 * `premultipliedAlpha` — that refusal is a plain unblended quad, i.e. a white
 * square. Nothing in the node pipeline premultiplies for us, so the colour
 * node carries `tint × mask` itself; the blend then works out to
 * `dst × mix(1, tint, mask)`, leaving the ground untouched outside the sole.
 */
let sharedPrintMaterial: MeshBasicNodeMaterial | null = null

function printMaterial(): MeshBasicNodeMaterial {
  if (sharedPrintMaterial) return sharedPrintMaterial
  const mat = new MeshBasicNodeMaterial()
  mat.transparent = true
  mat.depthWrite = false
  mat.blending = THREE.MultiplyBlending
  mat.premultipliedAlpha = true
  mat.polygonOffset = true
  mat.polygonOffsetFactor = -2
  mat.polygonOffsetUnits = -2

  const texNode: N = texture(printTexture())
  const opacity: N = attribute(FOOTPRINT_OPACITY_ATTR, 'float')
  const mask: N = texNode.a.mul(opacity)
  mat.colorNode = vec3(...WET_TINT).mul(mask)
  mat.opacityNode = mask

  sharedPrintMaterial = mat
  return mat
}

/**
 * Wet footprints trailing the local player while the `wet` debuff is up
 * (doc/DEBUFF.md). Pure client-side dressing — the caller decides when to
 * `emit`, this only ages the prints out.
 *
 * Prints lie flat at the walker's own feet Y, so house storeys and dungeon
 * floors work without a terrain sample. The instance basis is built by hand
 * (`makeBasis`) rather than from a yaw angle: the quad's +V must point along
 * the walking direction, and a basis says that outright.
 */
export class WetFootprints {
  readonly group = new THREE.Group()
  private pool: Footprint[]
  private mesh: THREE.InstancedMesh
  private opacityAttr: THREE.InstancedBufferAttribute
  private next = 0
  /** Live prints, so a client that never gets wet draws nothing at all. */
  private live = 0
  private readonly forward = new THREE.Vector3()
  private readonly right = new THREE.Vector3()
  private readonly up = new THREE.Vector3(0, 1, 0)
  private readonly position = new THREE.Vector3()
  private readonly matrix = new THREE.Matrix4()
  private readonly zeroMatrix = new THREE.Matrix4().makeScale(0, 0, 0)

  constructor() {
    this.group.name = 'wetFootprints'
    this.pool = Array.from({ length: MAX_FOOTPRINTS }, () => ({
      age: PRINT_LIFETIME_S,
      strength: 0,
    }))

    const geom = new THREE.PlaneGeometry(1, 1)
    this.opacityAttr = new THREE.InstancedBufferAttribute(
      new Float32Array(MAX_FOOTPRINTS),
      1
    )
    geom.setAttribute(FOOTPRINT_OPACITY_ATTR, this.opacityAttr)

    this.mesh = new THREE.InstancedMesh(geom, printMaterial(), MAX_FOOTPRINTS)
    this.mesh.count = 0
    this.mesh.frustumCulled = false
    this.mesh.castShadow = false
    this.mesh.receiveShadow = false
    this.mesh.renderOrder = 1
    for (let i = 0; i < MAX_FOOTPRINTS; i++) {
      this.mesh.setMatrixAt(i, this.zeroMatrix)
    }
    this.group.add(this.mesh)
  }

  /**
   * Stamp one print at the feet. `dirX`/`dirZ` is the (unnormalized) walking
   * direction; `side` is +1 for the right foot, −1 for the left.
   */
  emit(
    x: number,
    y: number,
    z: number,
    dirX: number,
    dirZ: number,
    side: number,
    strength: number
  ) {
    const len = Math.hypot(dirX, dirZ)
    if (len < 1e-4) return
    this.forward.set(
      (dirX / len) * PRINT_LENGTH_M,
      0,
      (dirZ / len) * PRINT_LENGTH_M
    )
    // right = forward × up, in the XZ plane.
    this.right.set(-dirZ / len, 0, dirX / len)
    this.position.set(
      x + this.right.x * FOOT_OFFSET_M * side,
      y + GROUND_LIFT_M,
      z + this.right.z * FOOT_OFFSET_M * side
    )
    this.right.multiplyScalar(PRINT_WIDTH_M)

    const slot = this.next
    this.next = (this.next + 1) % MAX_FOOTPRINTS
    const print = this.pool[slot]
    if (print.age >= PRINT_LIFETIME_S) this.live++
    // The ring only grows the drawn range; a full lap keeps the whole pool
    // drawn until the last print in it has dried.
    this.mesh.count = Math.max(this.mesh.count, slot + 1)
    print.age = 0
    print.strength = strength
    // A print never moves again, so the matrix is written once here and the
    // per-frame loop only touches opacity.
    this.matrix.makeBasis(this.right, this.forward, this.up)
    this.matrix.setPosition(this.position)
    this.mesh.setMatrixAt(slot, this.matrix)
    this.mesh.instanceMatrix.needsUpdate = true
  }

  /** Age every live print; `deltaTime` in seconds. */
  update(deltaTime: number) {
    if (this.live === 0) return
    const dt = Math.min(deltaTime, 0.1)
    const opacityArr = this.opacityAttr.array as Float32Array
    let dirty = false

    let matricesDirty = false

    for (let i = 0; i < this.pool.length; i++) {
      const print = this.pool[i]
      if (print.age >= PRINT_LIFETIME_S) continue
      print.age += dt
      dirty = true
      if (print.age >= PRINT_LIFETIME_S) {
        opacityArr[i] = 0
        this.mesh.setMatrixAt(i, this.zeroMatrix)
        matricesDirty = true
        this.live--
        continue
      }
      // Damp at full strength for a while, then evaporate.
      const t = print.age / PRINT_LIFETIME_S
      const fade = t < PRINT_HOLD ? 1 : (1 - t) / (1 - PRINT_HOLD)
      opacityArr[i] = print.strength * fade * fade
    }

    if (matricesDirty) this.mesh.instanceMatrix.needsUpdate = true
    if (dirty) this.opacityAttr.needsUpdate = true
    if (this.live === 0) this.mesh.count = 0
  }

  dispose() {
    this.mesh.geometry.dispose()
    this.group.remove(this.mesh)
  }
}

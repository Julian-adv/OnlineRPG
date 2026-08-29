import * as THREE from 'three'
import { MeshBasicNodeMaterial } from 'three/webgpu'
import type { PlayerDamageInfo, PlayerGoldInfo } from '../stores/gameStore'
import { drawOutlinedText } from '../utils/textBadge'

const CANVAS_W = 512
const CANVAS_H = 128
const PX_FONT = 64
const FONT = `${PX_FONT}px sans-serif`
const PIXELS_PER_UNIT = 256
const OUTLINE_PX = 7
const MAX_SLOTS = 48
const LIFE_S = 1
const RISE_M_PER_S = 0.75

interface Slot {
  mesh: THREE.Mesh<THREE.PlaneGeometry, MeshBasicNodeMaterial>
  texture: THREE.CanvasTexture
  ctx: CanvasRenderingContext2D
  owner: DamageTextEmitter | null
  life: number
  yOffset: number
}

/** Shared billboard label pool; fixed-size canvases so textures upload in
 *  place and nothing is allocated per hit. `group` is mounted by GameScene.
 *  (TextLabel still reallocates per resize and never disposes — see its
 *  onDestroy.) */
class DamageTextPool {
  readonly group = new THREE.Group()
  private readonly geometry = new THREE.PlaneGeometry(
    CANVAS_W / PIXELS_PER_UNIT,
    CANVAS_H / PIXELS_PER_UNIT
  )
  private readonly slots: Slot[] = []
  /** Next slot to steal when every slot is live. */
  private cursor = 0

  constructor() {
    this.group.name = 'damage-text'
    for (let i = 0; i < MAX_SLOTS; i++) this.slots.push(this.createSlot())
  }

  acquire(owner: DamageTextEmitter): Slot {
    let slot = this.slots.find((s) => s.owner === null)
    if (!slot) {
      slot = this.slots[this.cursor]
      this.cursor = (this.cursor + 1) % MAX_SLOTS
    }
    slot.owner = owner
    slot.mesh.visible = true
    return slot
  }

  free(slot: Slot) {
    slot.owner = null
    slot.mesh.visible = false
  }

  private createSlot(): Slot {
    const canvas = document.createElement('canvas')
    canvas.width = CANVAS_W
    canvas.height = CANVAS_H
    const texture = new THREE.CanvasTexture(canvas)
    texture.colorSpace = THREE.SRGBColorSpace
    texture.minFilter = THREE.LinearFilter
    texture.magFilter = THREE.LinearFilter
    const material = new MeshBasicNodeMaterial()
    material.map = texture
    material.transparent = true
    material.depthWrite = false
    material.polygonOffset = true
    material.polygonOffsetFactor = -1
    material.polygonOffsetUnits = -1
    const mesh = new THREE.Mesh(this.geometry, material)
    mesh.visible = false
    mesh.frustumCulled = false
    this.group.add(mesh)
    return {
      mesh,
      texture,
      ctx: canvas.getContext('2d')!,
      owner: null,
      life: 0,
      yOffset: 0,
    }
  }
}

export const damageTextPool = new DamageTextPool()

export interface DamageTextInfos {
  damage?: PlayerDamageInfo
  regen?: PlayerDamageInfo
  gold?: PlayerGoldInfo
}

/** One entity's floating numbers: spawns a label whenever an info trigger
 *  advances and animates only the slots it owns. */
export class DamageTextEmitter {
  private slots: Slot[] = []
  private lastDamageTrigger = 0
  private lastRegenTrigger = 0
  private lastGoldTrigger = 0

  update(
    deltaTime: number,
    baseX: number,
    baseY: number,
    baseZ: number,
    camera: THREE.Camera,
    infos: DamageTextInfos,
    yOffset: number
  ) {
    const { damage, regen, gold } = infos
    if (damage && damage.trigger !== this.lastDamageTrigger) {
      this.lastDamageTrigger = damage.trigger
      this.spawn(
        damage.hit ? `${damage.damage}` : 'Miss',
        damage.hit ? '#ff4d4d' : '#a0aec0',
        yOffset
      )
    }
    if (regen && regen.trigger !== this.lastRegenTrigger) {
      this.lastRegenTrigger = regen.trigger
      this.spawn(`+${regen.damage}`, '#48bb78', yOffset)
    }
    if (gold && gold.trigger !== this.lastGoldTrigger) {
      this.lastGoldTrigger = gold.trigger
      this.spawn(`+${gold.amount} copper`, '#f6c453', yOffset)
    }
    if (this.slots.length === 0) return
    let live = 0
    for (const slot of this.slots) {
      // Stolen by the pool while we were not looking
      if (slot.owner !== this) continue
      slot.life -= deltaTime
      if (slot.life <= 0) {
        damageTextPool.free(slot)
        continue
      }
      this.slots[live++] = slot
      slot.mesh.material.opacity = Math.min(1, slot.life * 2)
      slot.mesh.position.set(baseX, baseY, baseZ)
      slot.mesh.quaternion.copy(camera.quaternion)
      slot.mesh.translateY(slot.yOffset + (LIFE_S - slot.life) * RISE_M_PER_S)
    }
    this.slots.length = live
  }

  dispose() {
    for (const slot of this.slots) {
      if (slot.owner === this) damageTextPool.free(slot)
    }
    this.slots.length = 0
  }

  private spawn(text: string, color: string, yOffset: number) {
    const slot = damageTextPool.acquire(this)
    const { ctx } = slot
    ctx.clearRect(0, 0, CANVAS_W, CANVAS_H)
    ctx.font = FONT
    const width = ctx.measureText(text).width
    const maxWidth = CANVAS_W - OUTLINE_PX * 2
    if (width > maxWidth) {
      ctx.font = `${Math.floor((PX_FONT * maxWidth) / width)}px sans-serif`
    }
    drawOutlinedText(ctx, text, CANVAS_W / 2, CANVAS_H / 2, {
      color,
      outlineColor: '#000000',
      outlineWidth: OUTLINE_PX,
    })
    slot.texture.needsUpdate = true
    slot.life = LIFE_S
    slot.yOffset = yOffset
    slot.mesh.material.opacity = 1
    this.slots.push(slot)
  }
}

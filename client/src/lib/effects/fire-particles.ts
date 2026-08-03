import * as THREE from 'three'
import { MeshBasicNodeMaterial } from 'three/webgpu'
import { attribute, texture } from 'three/tsl'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type N = any

const PARTICLE_OPACITY_ATTR = 'aFireOpacity'

interface FireParticle {
  alive: boolean
  age: number
  maxAge: number
  x: number
  y: number
  z: number
  vx: number
  vy: number
  vz: number
  baseScale: number
  phase: number
}

/** Per-source tuning; `spawn` fills in position/velocity/lifetime fields. */
interface FireTuning {
  maxParticles: number
  spawnRate: number
  particleSize: number
  buoyancy: number
  turbulence: { amp: number; fx: number; fz: number }
  spawn: (p: FireParticle, cx: number, cy: number, cz: number) => void
}

/** Soft radial gradient, one canvas texture shared by every fire. */
let sharedFireTexture: THREE.Texture | null = null

function fireTexture(): THREE.Texture {
  if (sharedFireTexture) return sharedFireTexture
  const size = 64
  const canvas = document.createElement('canvas')
  canvas.width = size
  canvas.height = size
  const ctx = canvas.getContext('2d')!

  const gradient = ctx.createRadialGradient(
    size / 2,
    size / 2,
    0,
    size / 2,
    size / 2,
    size / 2
  )
  gradient.addColorStop(0, 'rgba(255, 255, 255, 1)')
  gradient.addColorStop(0.3, 'rgba(255, 200, 80, 0.8)')
  gradient.addColorStop(0.6, 'rgba(255, 100, 20, 0.4)')
  gradient.addColorStop(1, 'rgba(255, 40, 0, 0)')

  ctx.fillStyle = gradient
  ctx.fillRect(0, 0, size, size)

  const tex = new THREE.CanvasTexture(canvas)
  tex.needsUpdate = true
  sharedFireTexture = tex
  return tex
}

/** One additive-blended node material shared by every fire mesh; each mesh
 *  supplies its own instanced opacity attribute under the same name. */
let sharedFireMaterial: MeshBasicNodeMaterial | null = null

function fireMaterial(): MeshBasicNodeMaterial {
  if (sharedFireMaterial) return sharedFireMaterial
  const mat = new MeshBasicNodeMaterial()
  mat.side = THREE.DoubleSide
  mat.transparent = true
  mat.depthWrite = false
  mat.blending = THREE.AdditiveBlending

  const texNode: N = texture(fireTexture())
  const opacity: N = attribute(PARTICLE_OPACITY_ATTR, 'float')
  mat.colorNode = texNode.rgb
  mat.opacityNode = texNode.a.mul(opacity)

  sharedFireMaterial = mat
  return mat
}

/**
 * Pooled billboard fire particles. The group lives in world space — feed the
 * emitter's world position via setOrigin(), then update() each frame.
 */
export class FireParticles {
  readonly group = new THREE.Group()
  private pool: FireParticle[]
  private mesh: THREE.InstancedMesh
  private opacityAttr: THREE.InstancedBufferAttribute
  private spawnAccumulator = 0
  private readonly tmpMatrix = new THREE.Matrix4()
  private readonly tmpPos = new THREE.Vector3()
  private readonly tmpScale = new THREE.Vector3()
  private readonly zeroMatrix = new THREE.Matrix4().makeScale(0, 0, 0)
  /** World-space emitter position, updated externally. */
  private readonly origin = new THREE.Vector3()

  constructor(private tuning: FireTuning) {
    this.pool = Array.from({ length: tuning.maxParticles }, () => ({
      alive: false,
      age: 0,
      maxAge: 0,
      x: 0,
      y: 0,
      z: 0,
      vx: 0,
      vy: 0,
      vz: 0,
      baseScale: 0,
      phase: 0,
    }))
    const geom = new THREE.PlaneGeometry(
      tuning.particleSize,
      tuning.particleSize
    )
    const opacityArray = new Float32Array(tuning.maxParticles)
    this.opacityAttr = new THREE.InstancedBufferAttribute(opacityArray, 1)
    geom.setAttribute(PARTICLE_OPACITY_ATTR, this.opacityAttr)

    this.mesh = new THREE.InstancedMesh(
      geom,
      fireMaterial(),
      tuning.maxParticles
    )
    this.mesh.frustumCulled = false
    this.mesh.castShadow = false
    this.mesh.receiveShadow = false

    for (let i = 0; i < tuning.maxParticles; i++) {
      this.mesh.setMatrixAt(i, this.zeroMatrix)
    }

    this.group.add(this.mesh)
  }

  /** Set the world-space emitter position (call before update). */
  setOrigin(worldPos: THREE.Vector3) {
    this.origin.copy(worldPos)
  }

  update(deltaTime: number, camera: THREE.Camera | undefined) {
    if (!camera) return

    const { buoyancy, turbulence } = this.tuning
    const dt = Math.min(deltaTime, 0.1)

    // Spawn new particles
    this.spawnAccumulator += dt
    const spawnInterval = 1 / this.tuning.spawnRate
    while (this.spawnAccumulator >= spawnInterval) {
      this.spawnAccumulator -= spawnInterval
      this.spawn(this.origin.x, this.origin.y, this.origin.z)
    }

    // Update existing particles
    const opacityArr = this.opacityAttr.array as Float32Array
    const camQuat = camera.quaternion

    for (let i = 0; i < this.pool.length; i++) {
      const p = this.pool[i]
      if (!p.alive) continue

      p.age += dt
      if (p.age >= p.maxAge) {
        p.alive = false
        this.mesh.setMatrixAt(i, this.zeroMatrix)
        opacityArr[i] = 0
        continue
      }

      const t = p.age / p.maxAge

      // Accelerate upward, add turbulence
      p.vy += buoyancy * dt
      p.vx += Math.sin(p.age * turbulence.fx + p.phase) * turbulence.amp * dt
      p.vz +=
        Math.cos(p.age * turbulence.fz + p.phase * 1.3) * turbulence.amp * dt

      p.x += p.vx * dt
      p.y += p.vy * dt
      p.z += p.vz * dt

      // Opacity: quick fade in, hold, fade out
      let opacity: number
      if (t < 0.1) opacity = t / 0.1
      else if (t > 0.4) opacity = 1.0 - (t - 0.4) / 0.6
      else opacity = 1.0

      opacityArr[i] = opacity

      // Scale shrinks over lifetime
      const scale = p.baseScale * (1.0 - t * 0.6)
      this.tmpPos.set(p.x, p.y, p.z)
      this.tmpScale.set(scale, scale, scale)
      this.tmpMatrix.compose(this.tmpPos, camQuat, this.tmpScale)
      this.mesh.setMatrixAt(i, this.tmpMatrix)
    }

    this.mesh.instanceMatrix.needsUpdate = true
    this.opacityAttr.needsUpdate = true
  }

  private spawn(cx: number, cy: number, cz: number) {
    const slot = this.pool.findIndex((p) => !p.alive)
    if (slot === -1) return

    const p = this.pool[slot]
    this.tuning.spawn(p, cx, cy, cz)
    p.phase = Math.random() * Math.PI * 2
    p.age = 0
    p.alive = true
  }

  dispose() {
    // Texture and material are shared across fires; only per-fire GPU
    // resources go.
    this.mesh.geometry.dispose()
    if (this.group.parent) {
      this.group.parent.remove(this.group)
    }
  }
}

/** Flame at a torch tip. */
export class TorchFireParticles extends FireParticles {
  constructor() {
    super({
      maxParticles: 32,
      spawnRate: 24,
      particleSize: 0.12,
      buoyancy: 0.8,
      turbulence: { amp: 0.3, fx: 12, fz: 10 },
      spawn: (p, cx, cy, cz) => {
        const spread = 0.02
        p.x = cx + (Math.random() - 0.5) * spread
        p.y = cy + (Math.random() - 0.5) * spread
        p.z = cz + (Math.random() - 0.5) * spread
        p.vx = (Math.random() - 0.5) * 0.1
        p.vy = 0.2 + Math.random() * 0.15
        p.vz = (Math.random() - 0.5) * 0.1
        p.maxAge = 0.3 + Math.random() * 0.4
        p.baseScale = 0.8 + Math.random() * 0.5
      },
    })
  }
}

/** Wider, taller, denser flame rising from a campfire's bed of logs. */
export class CampfireFireParticles extends FireParticles {
  constructor() {
    const spread = 0.18
    super({
      maxParticles: 32,
      spawnRate: 22,
      particleSize: 0.16,
      buoyancy: 0.2,
      turbulence: { amp: 0.08, fx: 6, fz: 5 },
      spawn: (p, cx, cy, cz) => {
        // A disc-shaped bed: particles rise faster near the center.
        const r = Math.sqrt(Math.random()) * spread
        const a = Math.random() * Math.PI * 2
        p.x = cx + Math.cos(a) * r
        p.y = cy + Math.random() * 0.025
        p.z = cz + Math.sin(a) * r
        p.vx = (Math.random() - 0.5) * 0.035
        p.vy = 0.08 + (1 - r / spread) * 0.08 + Math.random() * 0.05
        p.vz = (Math.random() - 0.5) * 0.035
        p.maxAge = 0.7 + Math.random() * 0.65
        p.baseScale = 0.9 + Math.random() * 0.7
      },
    })
  }
}

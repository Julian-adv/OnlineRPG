import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import {
  attachCapeFit,
  capeMinZAt,
  createCapeRig,
  fitCapeToSkeleton,
  type CapeRig,
} from './cape-rig'

const OPTIONS = {
  topWidth: 0.4,
  bottomWidth: 0.55,
  length: 0.9,
  segments: 5,
}

/** Rig hanging off a mover group, standing in for the spine bone. */
function makeRig(): { rig: CapeRig; mover: THREE.Group } {
  const mover = new THREE.Group()
  const rig = createCapeRig(OPTIONS)
  mover.add(rig.root)
  return { rig, mover }
}

function settle(rig: CapeRig, seconds: number) {
  for (let i = 0; i < Math.round(seconds * 60); i++) rig.update(1 / 60, null)
}

/** Chain tip world positions, one per chain. */
function tips(rig: CapeRig): THREE.Vector3[] {
  rig.root.updateMatrixWorld(true)
  const bones = rig.mesh.skeleton.bones
  const segments = OPTIONS.segments
  const out: THREE.Vector3[] = []
  for (let c = 0; c * segments < bones.length; c++) {
    const last = bones[c * segments + segments - 1]
    out.push(last.getWorldPosition(new THREE.Vector3()))
  }
  return out
}

/** Back-surface depth of the stand-in body, per height. The bulge between 0.9
 *  and 1.1 m is the buttocks. */
function bodyDepth(y: number): number {
  if (y > 1.2) return 0.15
  if (y >= 0.9) return 0.22
  return 0.1
}

/** Minimal stand-in for the character rigs: the model faces `facing`, whose
 *  left arm sits at +right of the body and whose spine is rotated with it. The
 *  skinned body mesh gives `fitCapeToSkeleton` a silhouette to measure. */
function makeCharacter(facing: THREE.Vector3): {
  root: THREE.Group
  spine: THREE.Bone
} {
  const root = new THREE.Group()
  const character = new THREE.Group()
  character.quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, 1), facing)
  root.add(character)

  const spine = new THREE.Bone()
  spine.name = 'Spine2'
  spine.position.set(0, 1.43, 0)
  character.add(spine)

  // In these rigs the left arm is at +x when the body faces +z.
  const leftArm = new THREE.Bone()
  leftArm.name = 'LeftArm'
  leftArm.position.set(0.18, 0.07, 0)
  const rightArm = new THREE.Bone()
  rightArm.name = 'RightArm'
  rightArm.position.set(-0.18, 0.07, 0)
  spine.add(leftArm, rightArm)

  // Body shell: a back surface at -z (the model faces +z before `facing` is
  // applied) plus a front surface, sampled every 5 cm of height.
  const verts: number[] = []
  for (let y = 0.4; y <= 1.65; y += 0.05) {
    for (const x of [-0.15, 0, 0.15]) {
      verts.push(x, y, -bodyDepth(y), x, y, 0.1)
    }
  }
  const geometry = new THREE.BufferGeometry()
  geometry.setAttribute('position', new THREE.Float32BufferAttribute(verts, 3))
  const n = verts.length / 3
  geometry.setAttribute(
    'skinIndex',
    new THREE.Uint16BufferAttribute(new Uint16Array(n * 4), 4)
  )
  const weights = new Float32Array(n * 4)
  for (let i = 0; i < n; i++) weights[i * 4] = 1
  geometry.setAttribute(
    'skinWeight',
    new THREE.Float32BufferAttribute(weights, 4)
  )

  const body = new THREE.SkinnedMesh(geometry, new THREE.MeshBasicMaterial())
  character.add(body)
  // Bind only once the bones have world matrices, or the skeleton's inverses —
  // which is where the fit reads the bind pose from — come out as identity.
  root.updateMatrixWorld(true)
  body.bind(new THREE.Skeleton([spine, leftArm, rightArm]))

  root.updateMatrixWorld(true)
  return { root, spine }
}

/** Every simulated cloth point, chain by chain: the bone origins plus the hem
 *  point extrapolated off the last bone. */
function capePoints(rig: CapeRig, segments: number): THREE.Vector3[] {
  rig.root.updateMatrixWorld(true)
  const bones = rig.mesh.skeleton.bones
  const out: THREE.Vector3[] = []
  for (let c = 0; c * segments < bones.length; c++) {
    for (let i = 0; i < segments; i++) {
      out.push(bones[c * segments + i].getWorldPosition(new THREE.Vector3()))
    }
    const last = bones[c * segments + segments - 1]
    const down = new THREE.Vector3(0, -1, 0).applyQuaternion(
      last.getWorldQuaternion(new THREE.Quaternion())
    )
    const segLen = bones[c * segments + 1].position.length()
    out.push(
      last.getWorldPosition(new THREE.Vector3()).addScaledVector(down, segLen)
    )
  }
  return out
}

describe('cape fit', () => {
  it.each([
    ['+z', new THREE.Vector3(0, 0, 1)],
    ['-z', new THREE.Vector3(0, 0, -1)],
    ['+x', new THREE.Vector3(1, 0, 0)],
  ])('hangs behind a character facing %s', (_label, facing) => {
    const { root, spine } = makeCharacter(facing)
    const fit = fitCapeToSkeleton(root)
    if (!fit) throw new Error('expected a fit')

    const rig = attachCapeFit(fit)
    root.updateMatrixWorld(true)

    // The rig's +z must face away from the character, and the collar must sit
    // just clear of the back at collar height — not inside the torso, and not
    // standing off it either.
    const capeBack = new THREE.Vector3(0, 0, 1).applyQuaternion(
      rig.root.getWorldQuaternion(new THREE.Quaternion())
    )
    expect(capeBack.dot(facing)).toBeLessThan(-0.99)

    const anchor = rig.root.getWorldPosition(new THREE.Vector3())
    const spineWorld = spine.getWorldPosition(new THREE.Vector3())
    const collarDepth = -anchor.clone().sub(spineWorld).dot(facing)
    expect(collarDepth).toBeGreaterThan(bodyDepth(anchor.y))
    expect(collarDepth).toBeLessThan(bodyDepth(anchor.y) + 0.04)
    expect(fit.parent).toBe(spine)

    const capeUp = new THREE.Vector3(0, 1, 0).applyQuaternion(
      rig.root.getWorldQuaternion(new THREE.Quaternion())
    )
    expect(capeUp.y).toBeGreaterThan(0.99)
  })

  it('sizes itself from the arm spread and torso height', () => {
    const { root } = makeCharacter(new THREE.Vector3(0, 0, 1))
    const fit = fitCapeToSkeleton(root)
    if (!fit) throw new Error('expected a fit')

    expect(fit.options.topWidth).toBeGreaterThan(0.25)
    expect(fit.options.bottomWidth).toBeGreaterThan(fit.options.topWidth)
    expect(fit.options.length).toBeGreaterThan(0.7)
    expect(fit.options.length).toBeLessThan(1.1)
  })

  it('sinks the collar by the bias without freeing the buttocks', () => {
    const { root, spine } = makeCharacter(new THREE.Vector3(0, 0, 1))
    const plain = fitCapeToSkeleton(root)
    const sunk = fitCapeToSkeleton(root, 0.1)
    if (!plain || !sunk) throw new Error('expected a fit')

    const depthOf = (fit: NonNullable<typeof plain>) => {
      const rig = attachCapeFit(fit)
      root.updateMatrixWorld(true)
      const collar = rig.root.getWorldPosition(new THREE.Vector3())
      const spineWorld = spine.getWorldPosition(new THREE.Vector3())
      rig.dispose()
      return -collar.clone().sub(spineWorld).z
    }

    expect(depthOf(plain) - depthOf(sunk)).toBeCloseTo(0.1, 3)

    // The bias fades with the drop, so the hem still clears the 0.22 m bulge:
    // at the collar the sheet may sit 0.1 m inside, a third of a metre down it
    // may not sit inside at all.
    const body = sunk.options.body!
    expect(capeMinZAt(body, 0)).toBeCloseTo(-0.02, 3)
    const bulgeDrop = 1.53 - 1.15
    const collarDepth = depthOf(sunk)
    expect(collarDepth + capeMinZAt(body, bulgeDrop)).toBeGreaterThan(0.219)
  })

  it('refuses rigs without the bones it needs', () => {
    const bare = new THREE.Group()
    bare.add(new THREE.Bone())
    expect(fitCapeToSkeleton(bare)).toBeNull()
  })

  it('keeps the buttocks from poking through, standing and walking', () => {
    const facing = new THREE.Vector3(0, 0, 1)
    const { root } = makeCharacter(facing)
    const fit = fitCapeToSkeleton(root)
    if (!fit) throw new Error('expected a fit')

    const rig = attachCapeFit(fit)

    const segments = fit.options.segments ?? 5
    const check = () => {
      for (const p of capePoints(rig, segments)) {
        const local = root.worldToLocal(p.clone())
        // Behind the body surface at its own height (-z is behind here).
        expect(-local.z).toBeGreaterThan(bodyDepth(local.y) - 1e-3)
      }
    }

    for (let i = 0; i < 120; i++) {
      rig.update(1 / 60, null)
      check()
    }

    // Walk forwards, sway the hips, and turn — the sheet is pressed against the
    // body the whole time.
    for (let i = 0; i < 240; i++) {
      root.position.z += 3 / 60
      root.position.y = Math.sin(i * 0.5) * 0.03
      root.rotation.y = Math.sin(i * 0.05) * 0.6
      rig.update(1 / 60, { windDirX: 0, windDirZ: 1, windStrength: 1 })
      check()
    }
  })
})

describe('cape rig', () => {
  it('settles hanging straight down when the wearer stands still', () => {
    const { rig } = makeRig()
    settle(rig, 3)

    // Heavy cloth comes to rest plumb: below the collar, neither swung forward
    // of it nor still drifting back.
    const local = rig.root.worldToLocal(tips(rig)[1].clone())
    expect(local.y).toBeLessThan(-OPTIONS.length * 0.75)
    expect(local.z).toBeGreaterThan(-1e-6)
    expect(local.z).toBeLessThan(0.02)
    expect(Math.abs(local.x)).toBeLessThan(0.05)
  })

  it('keeps its segment lengths while the wearer walks', () => {
    const { rig, mover } = makeRig()
    settle(rig, 1)

    for (let i = 0; i < 180; i++) {
      mover.position.z -= 3 / 60
      mover.position.y = Math.sin(i * 0.4) * 0.03
      rig.update(1 / 60, { windDirX: 1, windDirZ: 0, windStrength: 1 })
    }

    rig.root.updateMatrixWorld(true)
    const bones = rig.mesh.skeleton.bones
    const segLen = OPTIONS.length / OPTIONS.segments
    for (let i = 1; i < OPTIONS.segments; i++) {
      const a = bones[i - 1].getWorldPosition(new THREE.Vector3())
      const b = bones[i].getWorldPosition(new THREE.Vector3())
      expect(a.distanceTo(b)).toBeCloseTo(segLen, 5)
    }
  })

  it('trails behind a walking wearer, then falls back to rest', () => {
    const { rig, mover } = makeRig()
    settle(rig, 2)
    const restZ = rig.root.worldToLocal(tips(rig)[1].clone()).z

    for (let i = 0; i < 60; i++) {
      mover.position.z -= 4 / 60
      rig.update(1 / 60, null)
    }
    const movingZ = rig.root.worldToLocal(tips(rig)[1].clone()).z
    expect(movingZ).toBeGreaterThan(restZ + 0.05)

    settle(rig, 4)
    const settledZ = rig.root.worldToLocal(tips(rig)[1].clone()).z
    expect(settledZ).toBeLessThan(movingZ)
  })

  it('never swings forward past the collar plane', () => {
    const { rig, mover } = makeRig()
    settle(rig, 1)

    for (let i = 0; i < 240; i++) {
      mover.position.z += 5 / 60
      mover.rotation.y += 0.15
      rig.update(1 / 60, { windDirX: 0, windDirZ: -1, windStrength: 1 })
      for (const tip of tips(rig)) {
        expect(rig.root.worldToLocal(tip.clone()).z).toBeGreaterThan(-1e-3)
      }
    }
  })

  it('snaps to rest instead of stretching on a teleport', () => {
    const { rig, mover } = makeRig()
    settle(rig, 2)

    mover.position.set(400, 0, -400)
    rig.update(1 / 60, null)

    const local = rig.root.worldToLocal(tips(rig)[1].clone())
    expect(local.y).toBeLessThan(-OPTIONS.length * 0.75)
    expect(local.length()).toBeLessThan(OPTIONS.length * 1.05)
  })
})

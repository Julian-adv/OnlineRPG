import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import {
  computeCorpseGroundOffset,
  groundRetargetedClips,
  retargetAnimationsForCharacterModel,
} from './characterAnimationUtils'

// A one-bone skinned mesh whose vertices sit at the given local Ys, all fully
// weighted to the bone. `posedBoneY` moves the bone AFTER bind, standing in for
// a death clip that poses the skeleton away from its rest pose.
function makeSkinned(localYs: number[], posedBoneY = 0): THREE.Group {
  const n = localYs.length
  const positions = new Float32Array(n * 3)
  const skinIndex = new Uint16Array(n * 4)
  const skinWeight = new Float32Array(n * 4)
  for (let i = 0; i < n; i++) {
    positions[i * 3 + 1] = localYs[i]
    skinWeight[i * 4] = 1
  }
  const geom = new THREE.BufferGeometry()
  geom.setAttribute('position', new THREE.BufferAttribute(positions, 3))
  geom.setAttribute('skinIndex', new THREE.Uint16BufferAttribute(skinIndex, 4))
  geom.setAttribute(
    'skinWeight',
    new THREE.Float32BufferAttribute(skinWeight, 4)
  )

  const bone = new THREE.Bone()
  const mesh = new THREE.SkinnedMesh(geom, new THREE.MeshBasicMaterial())
  mesh.add(bone)
  mesh.bind(new THREE.Skeleton([bone]))

  const root = new THREE.Group()
  root.add(mesh)
  bone.position.y = posedBoneY
  root.updateMatrixWorld(true)
  return root
}

const CLEARANCE = 0.01

describe('computeCorpseGroundOffset', () => {
  it('grounds on the lowest vertex, wherever the body sits', () => {
    // One vertex dangling to -0.50 m (like a tail tip) below a body at 0.30 m.
    const root = makeSkinned([-0.5, ...new Array(100).fill(0.3)])
    expect(computeCorpseGroundOffset(root)).toBeCloseTo(0.5 + CLEARANCE, 5)
  })

  it('rests a uniformly-raised corpse on the floor', () => {
    const root = makeSkinned(new Array(100).fill(0.25))
    expect(computeCorpseGroundOffset(root)).toBeCloseTo(-0.25 + CLEARANCE, 5)
  })

  it('measures the current pose, not the bind pose', () => {
    const flat = new Array(100).fill(0.2)
    const rest = computeCorpseGroundOffset(makeSkinned(flat, 0))
    const posed = computeCorpseGroundOffset(makeSkinned(flat, 0.5))
    expect(posed).toBeCloseTo(rest - 0.5, 5)
  })

  it('returns 0 when there is no skinned geometry', () => {
    const root = new THREE.Group()
    root.add(
      new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial())
    )
    expect(computeCorpseGroundOffset(root)).toBe(0)
  })
})

// A rig whose bone offsets are all `scale` times the 1 m reference. The
// animation packs are authored on a 10x rig under an Armature scaled 0.1, so
// their raw bone positions are 10x anything rigged at 1:1 — playing them
// unretargeted stretches the limbs to that size.
function makeRig(scale: number, hipY = 1.0): THREE.Group {
  const hips = new THREE.Bone()
  hips.name = 'Hips'
  hips.position.set(0, hipY * scale, 0)
  const arm = new THREE.Bone()
  arm.name = 'LeftArm'
  arm.position.set(0, 0.25 * scale, 0)
  const hand = new THREE.Bone()
  hand.name = 'LeftHand'
  hand.position.set(0, 0.3 * scale, 0)
  hips.add(arm)
  arm.add(hand)

  const bones = [hips, arm, hand]
  const positions = new Float32Array(bones.length * 3)
  const skinIndex = new Uint16Array(bones.length * 4)
  const skinWeight = new Float32Array(bones.length * 4)
  for (let i = 0; i < bones.length; i++) {
    positions[i * 3 + 1] = i * 0.25 * scale
    skinIndex[i * 4] = i
    skinWeight[i * 4] = 1
  }
  const geom = new THREE.BufferGeometry()
  geom.setAttribute('position', new THREE.BufferAttribute(positions, 3))
  geom.setAttribute('skinIndex', new THREE.Uint16BufferAttribute(skinIndex, 4))
  geom.setAttribute(
    'skinWeight',
    new THREE.Float32BufferAttribute(skinWeight, 4)
  )

  const mesh = new THREE.SkinnedMesh(geom, new THREE.MeshBasicMaterial())
  mesh.add(hips)
  mesh.bind(new THREE.Skeleton(bones))

  const root = new THREE.Group()
  root.add(mesh)
  root.updateMatrixWorld(true)
  return root
}

function packLikeSource(): THREE.Group {
  const root = makeRig(10)
  root.scale.setScalar(0.1)
  root.updateMatrixWorld(true)
  return root
}

function packClip(name: string): THREE.AnimationClip {
  return new THREE.AnimationClip(name, 1, [
    new THREE.VectorKeyframeTrack(
      'Hips.position',
      [0, 1],
      [0, 10, 0, 0, 11, 0]
    ),
    new THREE.VectorKeyframeTrack(
      'LeftArm.position',
      [0, 1],
      [0, 2.5, 0, 0, 2.5, 0]
    ),
    new THREE.VectorKeyframeTrack(
      'LeftHand.position',
      [0, 1],
      [0, 3, 0, 0, 3, 0]
    ),
    new THREE.QuaternionKeyframeTrack(
      'LeftArm.quaternion',
      [0, 1],
      [0, 0, 0, 1, 0, 0, 0, 1]
    ),
  ])
}

describe('retargetAnimationsForCharacterModel', () => {
  it('drops the source rig bone positions that stretch the limbs', async () => {
    const [clip] = await retargetAnimationsForCharacterModel(
      makeRig(1),
      packLikeSource(),
      [packClip('slash1')]
    )

    const positionTracks = clip.tracks
      .filter((track) => track.name.endsWith('.position'))
      .map((track) => track.name)
    expect(positionTracks).toEqual(['Hips.position'])
    expect(clip.tracks.length).toBeGreaterThan(1)
  })

  it('keeps each target model on its own retarget', async () => {
    const source = packLikeSource()
    const [tall] = await retargetAnimationsForCharacterModel(
      makeRig(1, 1.2),
      source,
      [packClip('slash2')]
    )
    const [short] = await retargetAnimationsForCharacterModel(
      makeRig(1, 0.8),
      source,
      [packClip('slash2')]
    )

    const hipY = (clip: THREE.AnimationClip) =>
      clip.tracks.find((track) => track.name === 'Hips.position')?.values[1]
    expect(hipY(tall)).not.toBeCloseTo(hipY(short) as number, 3)
  })
})

describe('groundRetargetedClips', () => {
  const sunkClip = () =>
    new THREE.AnimationClip('dying', 1, [
      new THREE.VectorKeyframeTrack(
        'Hips.position',
        [0, 1],
        [0, -0.5, 0, 0, -0.4, 0]
      ),
    ])

  it('lifts a clip that plays below the floor', async () => {
    const rig = makeRig(1)
    const [grounded] = await groundRetargetedClips(rig, [sunkClip()])
    const hips = grounded.tracks.find((t) => t.name === 'Hips.position')!
    const raw = sunkClip().tracks[0].values

    const lift = hips.values[1] - raw[1]
    expect(lift).toBeGreaterThan(0)
    // A constant shift: the motion inside the clip is untouched.
    expect(hips.values[4] - raw[4]).toBeCloseTo(lift, 5)
  })

  it('leaves an already grounded clip alone', async () => {
    const rig = makeRig(1)
    const [once] = await groundRetargetedClips(rig, [sunkClip()])
    const [twice] = await groundRetargetedClips(rig, [once])
    const hipsOnce = once.tracks.find((t) => t.name === 'Hips.position')!
    const hipsTwice = twice.tracks.find((t) => t.name === 'Hips.position')!
    expect(hipsTwice.values[1]).toBeCloseTo(hipsOnce.values[1], 5)
  })
})

describe('groundRetargetedClips rest clip', () => {
  const fallingClip = () =>
    new THREE.AnimationClip('dying', 1, [
      new THREE.VectorKeyframeTrack(
        'Hips.position',
        [0, 0.5, 1],
        [0, 1, 0, 0, 0.2, 0, 0, -0.4, 0]
      ),
    ])

  async function lowestAtEnd(clip: THREE.AnimationClip, rig: THREE.Group) {
    const mesh = rig.children[0] as THREE.SkinnedMesh
    const mixer = new THREE.AnimationMixer(rig)
    mixer.clipAction(clip).play()
    mixer.setTime(clip.duration - 1e-4)
    rig.updateMatrixWorld(true)
    const v = new THREE.Vector3()
    let lowest = Infinity
    const position = mesh.geometry.getAttribute('position')
    for (let i = 0; i < position.count; i++) {
      v.fromBufferAttribute(position, i)
      mesh.applyBoneTransform(i, v)
      mesh.localToWorld(v)
      lowest = Math.min(lowest, v.y)
    }
    return lowest
  }

  it('lands the last pose on the floor', async () => {
    const rig = makeRig(1)
    const [grounded] = await groundRetargetedClips(rig, [fallingClip()], {
      restClip: 'dying',
    })
    expect(await lowestAtEnd(grounded, makeRig(1))).toBeCloseTo(0, 2)
  })

  it('sinks the last pose by the rest offset', async () => {
    const rig = makeRig(1)
    const [grounded] = await groundRetargetedClips(rig, [fallingClip()], {
      restClip: 'dying',
      restOffset: -0.05,
    })
    expect(await lowestAtEnd(grounded, makeRig(1))).toBeCloseTo(-0.05, 2)
  })
})

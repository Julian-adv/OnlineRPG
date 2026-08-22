import { describe, it, expect } from 'vitest'
import fs from 'node:fs'
import * as THREE from 'three'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import { attachCapeFit, fitCapeToSkeleton } from './cape-rig'
import {
  createCharacterModelRoot,
  retargetAnimationsForCharacterModel,
} from '../utils/characterAnimationUtils'

/** Re-pack a GLB without materials/textures, so GLTFLoader parses it headless
 *  (image decoding needs a DOM). Geometry, skins and clips are untouched. */
function stripped(path: string): ArrayBuffer {
  const buf = fs.readFileSync(path)
  const jsonLen = buf.readUInt32LE(12)
  const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString())
  const binChunk = buf.subarray(20 + jsonLen + 8)

  delete json.images
  delete json.textures
  delete json.samplers
  delete json.extensionsUsed
  delete json.extensionsRequired
  json.materials = [{ pbrMetallicRoughness: {} }]
  for (const mesh of json.meshes)
    for (const prim of mesh.primitives) prim.material = 0

  const jsonBytes = Buffer.from(JSON.stringify(json))
  const jsonAligned = Buffer.concat([
    jsonBytes,
    Buffer.alloc((4 - (jsonBytes.length % 4)) % 4, 0x20),
  ])
  const header = Buffer.alloc(12)
  header.write('glTF', 0, 'ascii')
  header.writeUInt32LE(2, 4)
  header.writeUInt32LE(12 + 8 + jsonAligned.length + 8 + binChunk.length, 8)
  const jsonHeader = Buffer.alloc(8)
  jsonHeader.writeUInt32LE(jsonAligned.length, 0)
  jsonHeader.write('JSON', 4, 'ascii')
  const binHeader = Buffer.alloc(8)
  binHeader.writeUInt32LE(binChunk.length, 0)
  binHeader.writeUInt32LE(0x004e4942, 4)

  const out = Buffer.concat([
    header,
    jsonHeader,
    jsonAligned,
    binHeader,
    binChunk,
  ])
  return out.buffer.slice(out.byteOffset, out.byteOffset + out.byteLength)
}

function loadGltf(path: string): Promise<{
  scene: THREE.Object3D
  animations: THREE.AnimationClip[]
}> {
  return new Promise((resolve, reject) => {
    new GLTFLoader().parse(stripped(path), '', resolve, reject)
  })
}

/** Deepest back-surface depth within a band of `y`, skinned through the pose the
 *  rig is in — the surface a player actually sees. */
function posedSurfaceDepth(
  root: THREE.Object3D,
  back: THREE.Vector3,
  y: number,
  skip: THREE.Object3D
): number {
  const band = 0.06
  const v = new THREE.Vector3()
  let deepest = -Infinity
  root.traverse((obj) => {
    if (!(obj instanceof THREE.SkinnedMesh) || !obj.skeleton) return
    if (obj === skip) return
    const position = obj.geometry.getAttribute('position')
    for (let i = 0; i < position.count; i++) {
      v.fromBufferAttribute(position, i)
      obj.applyBoneTransform(i, v)
      obj.localToWorld(v)
      if (Math.abs(v.y - y) > band) continue
      deepest = Math.max(deepest, v.dot(back))
    }
  })
  return deepest
}

const CLIPS = ['idle1', 'walk', 'jog']
const RIGS = ['knight', 'female_knight', 'barbarian']

// The models live on Hugging Face, not in git — CI checks out without them.
const MODELS = [
  'public/models/animations/locomotion.glb',
  ...RIGS.map((rig) => `public/models/characters/${rig}.glb`),
]

describe.skipIf(MODELS.some((path) => !fs.existsSync(path)))(
  'cape fit on the real rigs',
  () => {
    it.each(RIGS)(
      'hugs %s through every locomotion pose',
      async (rig) => {
        const loco = await loadGltf('public/models/animations/locomotion.glb')
        const source = await loadGltf(`public/models/characters/${rig}.glb`)
        const { clonedScene, modelRoot } = createCharacterModelRoot(
          source.scene
        )
        modelRoot.updateMatrixWorld(true)

        const fit = fitCapeToSkeleton(clonedScene)
        expect(fit).not.toBeNull()
        if (!fit) return

        const cape = attachCapeFit(fit)

        for (const clipName of CLIPS) {
          const clip = loco.animations.find((c) => c.name === clipName)
          expect(clip, `${clipName} clip`).toBeDefined()
          const [retargeted] = await retargetAnimationsForCharacterModel(
            modelRoot,
            loco.scene,
            [clip!]
          )
          const mixer = new THREE.AnimationMixer(modelRoot)
          mixer.clipAction(retargeted).play()
          mixer.update(0.9)
          modelRoot.updateMatrixWorld(true)
          cape.update(1 / 60, null)

          const collar = cape.root.getWorldPosition(new THREE.Vector3())
          const back = new THREE.Vector3(0, 0, 1).applyQuaternion(
            cape.root.getWorldQuaternion(new THREE.Quaternion())
          )
          const surface = posedSurfaceDepth(
            modelRoot,
            back,
            collar.y,
            cape.mesh
          )
          const gap = collar.dot(back) - surface

          // Clear of the back, but nowhere near the head-sized standoff that a
          // pose-dependent fit used to bake in.
          expect(gap, `${rig}/${clipName} collar gap`).toBeGreaterThan(0)
          expect(gap, `${rig}/${clipName} collar gap`).toBeLessThan(0.045)
          mixer.stopAllAction()
        }
      },
      20000
    )
  }
)

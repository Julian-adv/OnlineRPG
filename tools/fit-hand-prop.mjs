#!/usr/bin/env node
/**
 * Solve the hand-local rotation a two-handed prop needs so its long axis lands
 * under the off hand in a given animation clip — the number that otherwise has
 * to be eyeballed in-game one nudge at a time.
 *
 * The prop is authored with its long axis along +X, its face normal along +Z
 * and its origin on the held point (doc/assets/items.md records this per
 * asset). The tool takes the off hand into the held hand's local space on each
 * sampled frame, fits +X to that cloud of points, and rolls +Z toward the chest
 * normal. It then replays the one euler against every frame and reports how far
 * off the axis the off hand drifts — a fixed rotation cannot track a clip that
 * moves the hands relative to each other, so that residual is the honest floor.
 *
 * Both hands are taken at the point the fist closes, not at the wrist bone: the
 * held hand by `--offset` down the palm, the off hand from its index and thumb.
 * A wrist is the better part of a palm's width from the prop it is holding, and
 * aiming at one leaves the prop visibly short of the hand.
 *
 * Three knobs then make the art calls the fit cannot. All pivot on the off
 * hand's grip, so none of them costs that hand its contact:
 *
 *   --tilt   how steeply the prop hangs. Trades a held hand that walks across
 *            the prop's face for a far end that stays put.
 *   --push   lifts the held hand clear of the face, for a prop thick enough
 *            that the wrist would otherwise be inside it. Paid for out of
 *            `chest gap`: the prop retreats toward the torso one-for-one.
 *   --arm    swings the held arm away from the chest before fitting.
 *
 * `--arm` is a dead end kept because it looks like the obvious fix and is not:
 * the prop hangs off the hand, so the arm carries it along and the elbow's
 * position *relative to the face* only worsens. Measured on the mandolin, 11 cm
 * of swing moves the elbow from -0.164 to -0.252 and buys nothing. Freeing a
 * buried forearm needs a clip whose strumming arm is held off the body, or the
 * arm re-animated with IK — not an offset.
 *
 *   node tools/fit-hand-prop.mjs                       # mandolin / guitar_playing
 *   node tools/fit-hand-prop.mjs --tilt 15             # neck up, body to the waist
 *   node tools/fit-hand-prop.mjs --prop weapons/x.glb --clip bowing \
 *        --pack social --char knight.glb
 *
 * It prints a rotation and a position for the attach site in PlayerModel.svelte
 * (the position is the plain palm offset only at --tilt 0 --push 0). Re-run it
 * when the clip is re-retargeted or the prop is re-exported.
 */
import { readFileSync } from 'fs'
import { resolve, dirname, basename } from 'path'
import { fileURLToPath } from 'url'
// tools/ has no package.json of its own; reach into the client's three.
import * as THREE from '../client/node_modules/three/build/three.module.js'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const PUBLIC = resolve(ROOT, 'client/public/models')

const argv = process.argv.slice(2)
const opt = (name, fallback) => {
  const i = argv.indexOf(`--${name}`)
  return i === -1 ? fallback : argv[i + 1]
}
const PROP = opt('prop', 'weapons/mandolin.glb')
const CLIP = opt('clip', 'guitar_playing')
const PACK = opt('pack', 'social')
const HELD_HAND = opt('hand', 'Right')
const OFF_HAND = HELD_HAND === 'Right' ? 'Left' : 'Right'
const OFFSET = new THREE.Vector3(
  ...opt('offset', '0,0.08,0').split(',').map(Number)
)
const SAMPLES = Number(opt('samples', 8))
// Swing the prop about its own face normal, pivoting where the off hand grips
// so that hand stays on it. Positive lifts the far end and drops the near end —
// for an instrument, neck up and body toward the waist. The held hand slides
// across the face as it swings (never off it), which is the cost of the tilt.
const TILT = (Number(opt('tilt', 0)) * Math.PI) / 180
// Metres the held hand should ride in front of the prop's face rather than
// inside it. Rotating about the grip is what makes this free of off-hand
// contact; what it costs instead is `chest gap`, since the prop retreats toward
// the torso by the same amount.
const PUSH = Number(opt('push', 0))
// Metres of hand travel away from the chest, applied to the upper-arm track
// before fitting. See the header: this does not do what it looks like it does.
const ARM = Number(opt('arm', 0))
// Metres to raise the off hand's aim point, straight up in world space. The
// fist closes *around* a neck, so aiming at its centre threads the prop through
// the hand; the axis wants to ride above it, on the fingers. How far above is
// the prop's own half-thickness there, which the fit cannot know.
const LIFT = Number(opt('lift', 0))
// Every rig shares the normalized skeleton, so one euler covers them all; the
// spread across these is the check that it does.
const CHARS = opt(
  'char',
  'female_bard.glb,knight.glb,female_priest.glb,barbarian.glb,valkyrie.glb'
).split(',')

// ── GLB reading (JSON + BIN chunks, no glTF library) ────────────────────────

function parseGlb(path) {
  const buf = readFileSync(path)
  if (buf.readUInt32LE(0) !== 0x46546c67) throw new Error(`Not a GLB: ${path}`)
  const total = buf.readUInt32LE(8)
  let off = 12
  let json = null
  let bin = null
  while (off < total) {
    const len = buf.readUInt32LE(off)
    const type = buf.readUInt32LE(off + 4)
    const start = off + 8
    if (type === 0x4e4f534a) {
      json = JSON.parse(buf.toString('utf8', start, start + len))
    } else if (type === 0x004e4942) {
      bin = buf.subarray(start, start + len)
    }
    off = start + len
  }
  return { json, bin }
}

const COMPONENT = {
  5120: [Int8Array, 1, 127],
  5121: [Uint8Array, 1, 255],
  5122: [Int16Array, 2, 32767],
  5123: [Uint16Array, 2, 65535],
  5125: [Uint32Array, 4, 0],
  5126: [Float32Array, 4, 0],
}
const COMPONENTS_PER = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4, MAT4: 16 }

/** Accessor rows, de-normalizing the integer encodings glTF allows. */
function readAccessor(glb, index) {
  const acc = glb.json.accessors[index]
  const [Arr, bytes, scale] = COMPONENT[acc.componentType]
  const n = COMPONENTS_PER[acc.type]
  const view = glb.json.bufferViews[acc.bufferView]
  const base = (view.byteOffset || 0) + (acc.byteOffset || 0)
  const stride = view.byteStride || n * bytes
  const rows = []
  for (let i = 0; i < acc.count; i++) {
    const raw = Array.from(
      new Arr(glb.bin.buffer, glb.bin.byteOffset + base + i * stride, n)
    )
    rows.push(scale ? raw.map((v) => Math.max(v / scale, -1)) : raw)
  }
  return rows
}

// ── Clip: bone name -> rotation track ───────────────────────────────────────

const pack = parseGlb(resolve(PUBLIC, `animations/${PACK}.glb`))
const clip = pack.json.animations?.find((a) => a.name === CLIP)
if (!clip) {
  const names = (pack.json.animations || []).map((a) => a.name).join(', ')
  throw new Error(`Clip "${CLIP}" not in ${PACK}.glb — has: ${names}`)
}

const tracks = new Map()
let clipEnd = 0
for (const channel of clip.channels) {
  if (channel.target.path !== 'rotation') continue
  const sampler = clip.samplers[channel.sampler]
  const times = readAccessor(pack, sampler.input).map((t) => t[0])
  clipEnd = Math.max(clipEnd, times[times.length - 1])
  tracks.set(pack.json.nodes[channel.target.node].name, {
    times,
    quats: readAccessor(pack, sampler.output),
  })
}

function sampleQuat({ times, quats }, t) {
  let i = 0
  while (i < times.length - 1 && times[i + 1] < t) i++
  const j = Math.min(i + 1, times.length - 1)
  const span = times[j] - times[i]
  const a = new THREE.Quaternion(...quats[i])
  if (span <= 0) return a
  const alpha = Math.min(Math.max((t - times[i]) / span, 0), 1)
  return a.slerp(new THREE.Quaternion(...quats[j]), alpha)
}

// ── Prop extents ────────────────────────────────────────────────────────────

/** World-space bounding box of every mesh in a GLB, walked node by node. */
function boundingBox(glb) {
  const min = new THREE.Vector3(Infinity, Infinity, Infinity)
  const max = new THREE.Vector3(-Infinity, -Infinity, -Infinity)
  const walk = (i, parentMatrix) => {
    const node = glb.json.nodes[i]
    const local = node.matrix
      ? new THREE.Matrix4().fromArray(node.matrix)
      : new THREE.Matrix4().compose(
          new THREE.Vector3(...(node.translation || [0, 0, 0])),
          new THREE.Quaternion(...(node.rotation || [0, 0, 0, 1])),
          new THREE.Vector3(...(node.scale || [1, 1, 1]))
        )
    const world = new THREE.Matrix4().multiplyMatrices(parentMatrix, local)
    if (node.mesh != null) {
      for (const prim of glb.json.meshes[node.mesh].primitives) {
        const acc = glb.json.accessors[prim.attributes.POSITION]
        for (let corner = 0; corner < 8; corner++) {
          const p = new THREE.Vector3(
            corner & 1 ? acc.max[0] : acc.min[0],
            corner & 2 ? acc.max[1] : acc.min[1],
            corner & 4 ? acc.max[2] : acc.min[2]
          ).applyMatrix4(world)
          min.min(p)
          max.max(p)
        }
      }
    }
    for (const child of node.children || []) walk(child, world)
  }
  for (const root of glb.json.scenes[glb.json.scene || 0].nodes) {
    walk(root, new THREE.Matrix4())
  }
  return { min, max }
}

const propBox = boundingBox(parseGlb(resolve(PUBLIC, PROP)))
const tipX = propBox.max.x
const buttX = propBox.min.x
console.log(
  `prop ${PROP}  size ${(propBox.max.x - propBox.min.x).toFixed(3)} x ` +
    `${(propBox.max.y - propBox.min.y).toFixed(3)} x ` +
    `${(propBox.max.z - propBox.min.z).toFixed(3)} m, ` +
    `tip at +X ${tipX.toFixed(3)} m`
)
console.log(`clip ${CLIP} (${PACK}.glb), ${clipEnd.toFixed(2)}s\n`)

// ── Per-rig solve ───────────────────────────────────────────────────────────

const UP = new THREE.Vector3(0, 1, 0)
const ARM_BONE = `${HELD_HAND}Arm`

// Swinging the held arm away from the chest is the only way to make room for a
// prop the forearm would otherwise pass through — pushing the prop back instead
// just walks the off hand off it, one centimetre for one. The delta is a fixed
// rotation in the shoulder's frame, applied to the upper-arm track, so the whole
// arm below it swings rigidly and the elbow stays tucked while the hand comes
// out. Solved once on the first rig and reused: the rigs share a skeleton.
let armDelta = null

/** Leading eigenvector of the points' scatter matrix, by power iteration. */
function principalAxis(points) {
  let v = points[0].clone().normalize()
  for (let i = 0; i < 64; i++) {
    const next = new THREE.Vector3()
    for (const p of points) next.addScaledVector(p, p.dot(v))
    v = next.normalize()
  }
  return v
}

function solve(charFile) {
  const char = parseGlb(resolve(PUBLIC, `characters/${charFile}`))
  const nodes = char.json.nodes
  const indexByName = new Map(nodes.map((n, i) => [n.name, i]))
  const parentOf = new Array(nodes.length).fill(-1)
  nodes.forEach((n, i) => (n.children || []).forEach((c) => (parentOf[c] = i)))

  const worldMatrix = (index, t, posed = true) => {
    const chain = []
    for (let k = index; k !== -1; k = parentOf[k]) chain.push(k)
    const m = new THREE.Matrix4()
    for (let k = chain.length - 1; k >= 0; k--) {
      const node = nodes[chain[k]]
      const pos = new THREE.Vector3(...(node.translation || [0, 0, 0]))
      const scale = new THREE.Vector3(...(node.scale || [1, 1, 1]))
      let quat = new THREE.Quaternion(...(node.rotation || [0, 0, 0, 1]))
      if (node.matrix) {
        new THREE.Matrix4().fromArray(node.matrix).decompose(pos, quat, scale)
      }
      const track = posed && tracks.get(node.name)
      if (track) quat = sampleQuat(track, t)
      if (armDelta && posed && node.name === ARM_BONE) {
        quat = armDelta.clone().multiply(quat)
      }
      m.multiply(new THREE.Matrix4().compose(pos, quat, scale))
    }
    return m
  }
  const at = (name, t, posed = true) =>
    new THREE.Vector3().setFromMatrixPosition(
      worldMatrix(indexByName.get(name), t, posed)
    )

  // Which way does this rig face? The toes point forward in the rest pose, so
  // they fix the sign of (shoulder axis x up).
  const restForward = at(`${HELD_HAND}ToeBase`, 0, false)
    .sub(at(`${HELD_HAND}Foot`, 0, false))
    .setY(0)
    .normalize()
  const restRight = at('RightShoulder', 0, false)
    .sub(at('LeftShoulder', 0, false))
    .setY(0)
    .normalize()
  const forwardSign =
    new THREE.Vector3().crossVectors(restRight, UP).dot(restForward) >= 0
      ? 1
      : -1

  // The off hand grips with its fingers, not its wrist. Aim the prop's axis at
  // the tunnel the closed fist makes — halfway between the finger bases and the
  // thumb — or the prop passes a palm's width shy of the hand that holds it.
  // Index base and thumb are the only finger bones every rig carries — the
  // fuller hands add the other three, but those sit alongside the index and
  // barely move the midpoint, so this stays uniform across the roster.
  const gripBones = [`${OFF_HAND}HandIndex1`, `${OFF_HAND}HandThumb3`]
  const hasFingers = gripBones.every((b) => indexByName.has(b))
  const offGrip = (t) =>
    (hasFingers
      ? gripBones
          .reduce((sum, b) => sum.add(at(b, t)), new THREE.Vector3())
          .divideScalar(gripBones.length)
      : at(`${OFF_HAND}Hand`, t)
    ).addScaledVector(UP, LIFT)

  // Solve the arm swing on the middle frame: rotate the shoulder-to-hand vector
  // toward the chest normal, in the shoulder's own frame so the delta is a plain
  // pre-multiply on the upper-arm track.
  if (ARM && !armDelta) {
    const t = clipEnd / 2
    const shoulder = worldMatrix(indexByName.get(ARM_BONE), t)
    const toShoulder = new THREE.Matrix4().extractRotation(shoulder).invert()
    const reach = at(`${HELD_HAND}Hand`, t)
      .sub(new THREE.Vector3().setFromMatrixPosition(shoulder))
      .applyMatrix4(toShoulder)
    const forward = new THREE.Vector3()
      .crossVectors(at('RightShoulder', t).sub(at('LeftShoulder', t)), UP)
      .multiplyScalar(forwardSign)
      .normalize()
      .applyMatrix4(toShoulder)
    const swing = new THREE.Vector3()
      .crossVectors(reach, forward)
      .normalize()
      .negate()
    armDelta = new THREE.Quaternion().setFromAxisAngle(
      swing,
      Math.asin(Math.min(ARM / reach.length(), 1))
    )
    console.log(
      `arm swing: ${(ARM * 100).toFixed(0)} cm of hand travel = ` +
        `${((Math.asin(Math.min(ARM / reach.length(), 1)) * 180) / Math.PI).toFixed(1)}deg ` +
        `about (${swing.toArray().map((v) => v.toFixed(3))}) in the ${ARM_BONE} frame`
    )
    console.log(
      `  client: q.premultiply(new THREE.Quaternion().setFromAxisAngle(\n` +
        `    new THREE.Vector3(${swing
          .toArray()
          .map((v) => v.toFixed(4))
          .join(', ')}),\n` +
        `    ${Math.asin(Math.min(ARM / reach.length(), 1)).toFixed(4)}))\n`
    )
  }

  const frames = []
  for (let s = 0; s < SAMPLES; s++) {
    const t = (clipEnd * s) / SAMPLES
    const held = worldMatrix(indexByName.get(`${HELD_HAND}Hand`), t)
    const heldInverse = new THREE.Matrix4().copy(held).invert()
    const off = offGrip(t).applyMatrix4(heldInverse)

    // Long axis: from the held point toward the off hand.
    const axis = off.clone().sub(OFFSET).normalize()

    // Face normal: away from the chest, taken into hand-local space.
    const chestForward = new THREE.Vector3()
      .crossVectors(at('RightShoulder', t).sub(at('LeftShoulder', t)), UP)
      .multiplyScalar(forwardSign)
      .normalize()
      .applyMatrix4(new THREE.Matrix4().extractRotation(held).invert())

    frames.push({ t, off, axis, face: chestForward })
  }

  // One fixed axis serves the whole clip, so take the one with the least RMS
  // perpendicular error: the leading eigenvector of the off-hand scatter.
  const axis = principalAxis(frames.map((f) => f.off.clone().sub(OFFSET)))
  if (axis.dot(frames[0].axis) < 0) axis.negate()
  const meanFace = frames
    .reduce((acc, f) => acc.add(f.face), new THREE.Vector3())
    .normalize()
  const z = meanFace
    .sub(axis.clone().multiplyScalar(meanFace.dot(axis)))
    .normalize()
  const fitted = new THREE.Matrix4().makeBasis(
    axis,
    new THREE.Vector3().crossVectors(z, axis),
    z
  )

  // Where along the axis the off hand grips, averaged — the pivot the tilt
  // swings about, so tilting leaves that hand on the prop.
  const gripX =
    frames.reduce((sum, f) => sum + f.off.clone().sub(OFFSET).dot(axis), 0) /
    frames.length
  const meanOff = OFFSET.clone().addScaledVector(axis, gripX)

  // Lift the held hand `PUSH` clear of the face. Any rotation about the grip
  // keeps the off hand on the prop, so this is free of contact — the price is
  // paid in how the prop now sits. Take the smallest such rotation: swing the
  // hand's offset onto the z = PUSH plane along the shortest arc, keeping its
  // radius (which a rotation about the grip preserves anyway).
  const tilt = new THREE.Matrix4().makeRotationZ(TILT)
  const tilted = new THREE.Matrix4().multiplyMatrices(fitted, tilt)
  const arm = OFFSET.clone()
    .sub(meanOff)
    .applyMatrix4(new THREE.Matrix4().copy(tilted).invert())
  const reach = arm.length()
  if (PUSH > reach) {
    throw new Error(
      `--push ${PUSH} exceeds the ${reach.toFixed(3)} m from the off hand's ` +
        `grip to the held hand — there is no pose that far off the face`
    )
  }
  const flat = Math.hypot(arm.x, arm.y)
  const lifted = new THREE.Vector3(arm.x, arm.y, 0)
    .multiplyScalar(Math.sqrt(reach * reach - PUSH * PUSH) / flat)
    .setZ(PUSH)
  // Rotating the prop by S re-expresses the hand in prop space by S inverse, so
  // this maps the target back onto the current offset, not the other way round.
  const swing = new THREE.Quaternion().setFromUnitVectors(
    lifted.clone().normalize(),
    arm.clone().normalize()
  )
  const roll = 2 * Math.acos(Math.min(Math.abs(swing.w), 1))

  const rotation = tilted.multiply(
    new THREE.Matrix4().makeRotationFromQuaternion(swing)
  )
  const position = meanOff
    .clone()
    .sub(new THREE.Vector3(gripX, 0, 0).applyMatrix4(rotation))
  const mean = new THREE.Euler().setFromRotationMatrix(rotation, 'XYZ')

  // Replay the one transform: how far does the off hand drift off the axis?
  const finalAxis = new THREE.Vector3(1, 0, 0)
    .applyMatrix4(rotation)
    .normalize()
  const drift = frames.map((frame) => {
    const rel = frame.off.clone().sub(position)
    const along = rel.dot(finalAxis)
    return {
      along,
      off: rel.sub(finalAxis.clone().multiplyScalar(along)).length(),
    }
  })

  // How the prop hangs: the far end's rise over the grip end, and the butt's
  // height against the hip bone — the two things "diagonal enough?" comes down
  // to. Measured in world space on the middle frame.
  const t = frames[Math.floor(frames.length / 2)].t
  const propToWorld = new THREE.Matrix4()
    .multiplyMatrices(
      worldMatrix(indexByName.get(`${HELD_HAND}Hand`), t),
      new THREE.Matrix4().makeTranslation(position.x, position.y, position.z)
    )
    .multiply(rotation)
  const tip = new THREE.Vector3(tipX, 0, 0).applyMatrix4(propToWorld)
  const butt = new THREE.Vector3(buttX, 0, 0).applyMatrix4(propToWorld)

  // Clearance of the prop's far end in front of the chest. Lifting the held
  // hand off the face retreats the whole prop toward the torso, so this is the
  // budget any push spends — the chest surface is 10-12 cm out from Spine2 on
  // these rigs, so anything under that is inside the character.
  let chestGap = Infinity
  for (const frame of frames) {
    const spine = at('Spine2', frame.t)
    const forward = new THREE.Vector3()
      .crossVectors(
        at('RightShoulder', frame.t).sub(at('LeftShoulder', frame.t)),
        UP
      )
      .multiplyScalar(forwardSign)
      .normalize()
    const held = worldMatrix(indexByName.get(`${HELD_HAND}Hand`), frame.t)
    const toWorld = new THREE.Matrix4()
      .multiplyMatrices(
        held,
        new THREE.Matrix4().makeTranslation(position.x, position.y, position.z)
      )
      .multiply(rotation)
    const end = new THREE.Vector3(buttX, 0, 0).applyMatrix4(toWorld)
    chestGap = Math.min(chestGap, end.sub(spine).dot(forward))
  }
  const incline =
    (Math.atan2(tip.y - butt.y, Math.hypot(tip.x - butt.x, tip.z - butt.z)) *
      180) /
    Math.PI

  // Tilting slides the held hand across the prop's face. Report where it lands
  // in prop space — it has to stay within the model, or the hand plays air.
  const toProp = new THREE.Matrix4().copy(rotation).invert()
  const palm = OFFSET.clone().sub(position).applyMatrix4(toProp)
  // The wrist trails the palm and the elbow trails that, so they are the joints
  // that end up buried in a prop the hand is only nominally holding. Everything
  // behind the face (negative z) is inside or behind the prop.
  const wrist = position.clone().negate().applyMatrix4(toProp)
  const elbowZ = frames.map((frame) => {
    const held = worldMatrix(indexByName.get(`${HELD_HAND}Hand`), frame.t)
    return at(`${HELD_HAND}ForeArm`, frame.t)
      .applyMatrix4(new THREE.Matrix4().copy(held).invert())
      .sub(position)
      .applyMatrix4(toProp).z
  })
  const elbow = { min: Math.min(...elbowZ), max: Math.max(...elbowZ) }
  // Behind the face only matters where the prop actually is: an elbow past the
  // body's outline in x/y clears it however deep it sits.
  const elbowXY = frames.map((frame) => {
    const held = worldMatrix(indexByName.get(`${HELD_HAND}Hand`), frame.t)
    return at(`${HELD_HAND}ForeArm`, frame.t)
      .applyMatrix4(new THREE.Matrix4().copy(held).invert())
      .sub(position)
      .applyMatrix4(toProp)
  })
  elbow.y = {
    min: Math.min(...elbowXY.map((v) => v.y)),
    max: Math.max(...elbowXY.map((v) => v.y)),
  }
  elbow.x = {
    min: Math.min(...elbowXY.map((v) => v.x)),
    max: Math.max(...elbowXY.map((v) => v.x)),
  }

  return {
    mean,
    position,
    drift,
    incline,
    palm,
    wrist,
    elbow,
    roll: (roll * 180) / Math.PI,
    chestGap,
    buttOverHips: butt.y - at('Hips', t).y,
  }
}

const solved = CHARS.map((charFile) => ({ charFile, ...solve(charFile) }))

for (const s of solved) {
  const grip = s.drift.map((d) => d.along / tipX)
  const worst = Math.max(...s.drift.map((d) => d.off))
  console.log(
    `${basename(s.charFile).padEnd(20)} euler (${s.mean.x.toFixed(3)}, ` +
      `${s.mean.y.toFixed(3)}, ${s.mean.z.toFixed(3)})  ` +
      `off hand at ${(Math.min(...grip) * 100).toFixed(0)}-` +
      `${(Math.max(...grip) * 100).toFixed(0)}%, ` +
      `drift ${(worst * 100).toFixed(1)} cm, ` +
      `tips up ${s.incline.toFixed(0)}deg, ` +
      `butt ${(s.buttOverHips * 100).toFixed(0)} cm over the hips, ` +
      `palm (${s.palm.x.toFixed(2)}, ${s.palm.y.toFixed(2)}), ` +
      `roll ${s.roll.toFixed(0)}deg, ` +
      `wrist (${s.wrist.x.toFixed(2)}, ${s.wrist.y.toFixed(2)}, ${s.wrist.z.toFixed(2)}), elbow z ${s.elbow.min.toFixed(2)}..${s.elbow.max.toFixed(2)} ` +
      `y ${s.elbow.y.min.toFixed(2)}..${s.elbow.y.max.toFixed(2)} x ${s.elbow.x.min.toFixed(2)}..${s.elbow.x.max.toFixed(2)}, ` +
      `chest gap ${(s.chestGap * 100).toFixed(0)} cm`
  )
}

const axes = ['x', 'y', 'z'].map((k) => solved.map((s) => s.mean[k]))
const pos = ['x', 'y', 'z'].map((k) => solved.map((s) => s.position[k]))
const mid = (v) => v.reduce((a, b) => a + b) / v.length
console.log(
  `\nacross rigs: (${axes
    .map((v) => `${Math.min(...v).toFixed(3)}..${Math.max(...v).toFixed(3)}`)
    .join(', ')})`
)
console.log(
  `use: rotation new THREE.Euler(${axes.map((v) => mid(v).toFixed(3)).join(', ')})`
)
console.log(
  `     position new THREE.Vector3(${pos.map((v) => mid(v).toFixed(3)).join(', ')})`
)

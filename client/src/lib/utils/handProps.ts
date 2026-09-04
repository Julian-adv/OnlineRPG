// How a held item sits in the hand that holds it. Shared by the in-game player
// model and the character-select preview so both grip alike.

import * as THREE from 'three'
import { isRangedWeapon } from '../data/itemDefs'

/** Offset from the wrist bone toward the palm, so a prop looks gripped. */
const HAND_GRIP_OFFSET = new THREE.Vector3(0, 0.08, 0)

// In the fishing stance the hand bone's y-z plane runs forward-down to
// sideways, so a pure x pitch only swings the rod sideways; this euler
// points it forward and ~25° up (about 60° bent off the forearm).
const FISHING_ROD_ROTATION = new THREE.Euler(0, -Math.PI / 6, -Math.PI / 3)

// A bow rides the bow hand, so it needs its own seat in the palm rather than
// the shared grip offset. Fitted by eye in `tools/rig-importer` against the
// `bow_shoot` stance — the same bone-local position and XYZ euler that tool
// writes for monsters, which parents a weapon exactly as `attachWeaponModel`
// does here, so its numbers transplant unchanged. Re-fit if the clip or
// bow.glb changes.
const BOW_POSITION = new THREE.Vector3(0.01, 0.06, 0.04)
const BOW_ROTATION = new THREE.Euler((-13 * Math.PI) / 180, 0, 0)

// The along-bone offset is the one axis that cannot be a constant: it pushes
// the stave from the wrist out to the fingers, and these rigs are not one size.
// Forearm length is the proxy, not the hand's own geometry: a fur bracer
// weighted to the hand bone puts caveman's mesh reach at 0.225 m against
// knight's 0.062, and a rig with no finger bones (night_merchant) carries the
// whole palm in one bone. Forearms span a mere 0.197–0.267 m across all
// eighteen, with no such outlier. Fitted on knight.glb, whose forearm is
// 0.267 m.
const KNIGHT_FOREARM_METERS = 0.267
const BOW_FOREARM_RATIO = BOW_POSITION.y / KNIGHT_FOREARM_METERS

export const MANDOLIN_ITEM_DEF_ID = 'mandolin'

// The mandolin's origin sits on the strum point with the neck along +X and
// the soundboard facing +Z. Fitted to the guitar_playing clip by
// `tools/fit-hand-prop.mjs --tilt 15 --push 0.06 --lift 0.04`: 15° hangs the
// body down off the chest to the waist, the lift carries the neck up onto the
// fretting fingers instead of through the fist, and the push trades the sound
// box's depth in the torso against clearance for the strumming wrist, which
// the clip otherwise buries in it — 0.06 leaves the wrist 1 cm proud of the
// face. All three pivot on the fretting hand, so none of them costs the neck
// that grip. The position is no longer the plain palm offset because of them:
// it puts the origin back where the hand can hold it.
const MANDOLIN_ROTATION = new THREE.Euler(-2.413, -0.409, -0.353)
const MANDOLIN_POSITION = new THREE.Vector3(-0.03, 0.103, 0.126)

/** Which hand holds a main-hand item. A bow is drawn with the right hand, so
 *  the stave itself sits in the left — every other weapon leads with the right. */
export function mainHandBoneFor(
  itemDefId: string | null | undefined
): 'RightHand' | 'LeftHand' {
  return isRangedWeapon(itemDefId) ? 'LeftHand' : 'RightHand'
}

/** Pose a main-hand prop for the item it represents. `forearm` is this rig's
 *  forearm length (`forearmLength`); without it the bow falls back to the
 *  offset it was fitted at. */
export function poseMainHandProp(
  prop: THREE.Object3D,
  itemDefId: string,
  forearm?: number
) {
  prop.position.copy(HAND_GRIP_OFFSET)
  if (itemDefId === 'fishing_rod') {
    prop.rotation.copy(FISHING_ROD_ROTATION)
  } else if (itemDefId === MANDOLIN_ITEM_DEF_ID) {
    prop.position.copy(MANDOLIN_POSITION)
    prop.rotation.copy(MANDOLIN_ROTATION)
  } else if (isRangedWeapon(itemDefId)) {
    prop.position.copy(BOW_POSITION)
    if (forearm && forearm > 0) prop.position.y = forearm * BOW_FOREARM_RATIO
    prop.rotation.copy(BOW_ROTATION)
  }
}

const forearmCache = new Map<string, number>()

/** Elbow-to-wrist in metres, from the bind pose, for the arm `handBone` is on.
 *  0 when either bone is missing. Cached per `key` (the model's URL). */
export function forearmLength(
  root: THREE.Object3D,
  handBone: 'RightHand' | 'LeftHand',
  key: string
): number {
  const cached = forearmCache.get(key)
  if (cached !== undefined) return cached

  const foreArmBone = handBone === 'LeftHand' ? 'LeftForeArm' : 'RightForeArm'
  let length = 0
  root.traverse((child) => {
    const mesh = child as THREE.SkinnedMesh
    if (length > 0 || !mesh.isSkinnedMesh) return
    const at = (name: string) => {
      const index = mesh.skeleton.bones.findIndex((b) => b.name === name)
      if (index < 0) return null
      // The bind matrix is the inverse of what the skeleton stores.
      return new THREE.Vector3().setFromMatrixPosition(
        mesh.skeleton.boneInverses[index].clone().invert()
      )
    }
    const hand = at(handBone)
    const foreArm = at(foreArmBone)
    if (hand && foreArm) length = hand.distanceTo(foreArm)
  })

  forearmCache.set(key, length)
  return length
}
/** Pose a left-hand prop — shields and torches face the other way. */
export function poseOffHandProp(prop: THREE.Object3D) {
  prop.position.copy(HAND_GRIP_OFFSET)
  prop.rotation.y = Math.PI
}

/** Where the flame sits on a torch GLB that names no `torch_tip` empty. */
export const FALLBACK_TORCH_TIP_LOCAL_OFFSET = new THREE.Vector3(0.6, 0, 0)

/** Named tip empty baked into a prop GLB, or a fallback child at the given
 *  local offset — either way it rides the bone chain. */
export function resolveTipNode(
  prop: THREE.Object3D,
  name: string,
  fallbackOffset: THREE.Vector3
): THREE.Object3D {
  const found = prop.getObjectByName(name)
  if (found) return found
  const node = new THREE.Object3D()
  node.position.copy(fallbackOffset)
  prop.add(node)
  return node
}

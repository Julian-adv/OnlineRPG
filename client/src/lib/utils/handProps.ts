// How a held item sits in the hand that holds it. Shared by the in-game player
// model and the character-select preview so both grip alike.

import * as THREE from 'three'

/** Offset from the wrist bone toward the palm, so a prop looks gripped. */
const HAND_GRIP_OFFSET = new THREE.Vector3(0, 0.08, 0)

// In the fishing stance the hand bone's y-z plane runs forward-down to
// sideways, so a pure x pitch only swings the rod sideways; this euler
// points it forward and ~25° up (about 60° bent off the forearm).
const FISHING_ROD_ROTATION = new THREE.Euler(0, -Math.PI / 6, -Math.PI / 3)

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

/** Pose a right-hand prop for the item it represents. */
export function poseMainHandProp(prop: THREE.Object3D, itemDefId: string) {
  prop.position.copy(HAND_GRIP_OFFSET)
  if (itemDefId === 'fishing_rod') {
    prop.rotation.copy(FISHING_ROD_ROTATION)
  } else if (itemDefId === MANDOLIN_ITEM_DEF_ID) {
    prop.position.copy(MANDOLIN_POSITION)
    prop.rotation.copy(MANDOLIN_ROTATION)
  }
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

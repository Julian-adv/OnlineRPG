import * as THREE from 'three'
import * as SkeletonUtils from 'three/examples/jsm/utils/SkeletonUtils.js'
import { AnimationName } from '../types/animations'
import { loadGLB } from './gltfCache'
import { CHARACTER_ANIMATION_PACK_PATHS } from './modelPaths'

type AnimationSource = 'base' | 'locomotion' | 'combat_melee'

export interface OrderedAnimationSelection {
  name: AnimationName
  clip: THREE.AnimationClip
  source: AnimationSource
  fromFallback: boolean
}

export interface RetargetSourceScenes {
  base?: THREE.Object3D | null
  locomotion?: THREE.Object3D | null
  combatMelee?: THREE.Object3D | null
}

const ANIMATION_SOURCE_BY_NAME: Record<AnimationName, AnimationSource> = {
  [AnimationName.IDLE1]: 'locomotion',
  [AnimationName.IDLE2]: 'locomotion',
  [AnimationName.IDLE3]: 'locomotion',
  [AnimationName.IDLE4]: 'locomotion',
  [AnimationName.IDLE5]: 'locomotion',
  [AnimationName.WALK]: 'locomotion',
  [AnimationName.JOG]: 'locomotion',
  [AnimationName.RUN]: 'locomotion',
  [AnimationName.JUMP]: 'locomotion',
  [AnimationName.SLASH1]: 'combat_melee',
  [AnimationName.SLASH2]: 'combat_melee',
  [AnimationName.SLASH3]: 'combat_melee',
  [AnimationName.SLASH4]: 'combat_melee',
  [AnimationName.SLASH5]: 'combat_melee',
  [AnimationName.ATTACK1]: 'combat_melee',
  [AnimationName.ATTACK2]: 'combat_melee',
  [AnimationName.ATTACK3]: 'combat_melee',
  [AnimationName.ATTACK4]: 'combat_melee',
  [AnimationName.DYING]: 'combat_melee',
}

const RETARGET_TRACK_NAME_PATTERN = /^\.bones\[(.+?)\]\.(position|quaternion)$/
const HIP_BONE_CANDIDATES = [
  'Hips',
  'hips',
  'Hip',
  'hip',
  'Pelvis',
  'pelvis',
  'mixamorigHips',
] as const
const retargetedClipCache = new Map<string, THREE.AnimationClip>()
/** Top-level cache: source scene UUIDs + clip names → full result array.
 *  Avoids expensive SkeletonUtils.clone() when the same retarget was already done
 *  (e.g. character select → game scene for the same class). */
const retargetBatchCache = new Map<string, THREE.AnimationClip[]>()
const ENABLE_RUNTIME_BONE_RETARGETING = true

export function getGltfAnimations(gltf: unknown): THREE.AnimationClip[] {
  if (!gltf || typeof gltf !== 'object' || !('animations' in gltf)) return []

  const animations = (gltf as { animations?: unknown }).animations
  return Array.isArray(animations) ? (animations as THREE.AnimationClip[]) : []
}

export function createCharacterModelRoot(sourceScene: THREE.Object3D): {
  clonedScene: THREE.Object3D
  modelRoot: THREE.Group
} {
  const clonedScene = SkeletonUtils.clone(sourceScene) as THREE.Object3D
  const modelRoot = new THREE.Group()
  modelRoot.add(clonedScene)

  modelRoot.traverse((child) => {
    if (child instanceof THREE.Mesh) {
      child.castShadow = true
      child.receiveShadow = true
    }
  })

  return { clonedScene, modelRoot }
}

const FOOT_BONE_PATTERN = /foot|toe/i
/** A vertex counts as "on the sole" once foot/toe bones own a majority of it. */
const FOOT_INFLUENCE_THRESHOLD = 0.5
/** Tiny gap (m) kept between the soles and the floor so the contact shadow
 *  doesn't peter-pan where coplanar with the ground. */
const FOOT_GROUND_CLEARANCE = 0.005

/**
 * Y offset (metres) to add to a freshly-cloned model so its shoe soles rest
 * just above the floor plane (y = 0 at the model-root origin), with a hair of
 * ground clearance. Measured in the skeleton's current pose, so callers MUST
 * call this right after createCharacterModelRoot — i.e. in the deterministic
 * bind/rest pose, where both soles are planted — before any animation is
 * played. This replaces sampling an arbitrary first animation frame, which
 * (with a randomly-picked idle clip) produced a different lift every session
 * and left the character floating on flat ground.
 *
 * Only foot/toe-skinned vertices are considered, so skirts, capes and
 * not-yet-attached weapons can't pull the contact point below the soles.
 * Returns 0 when the model has no skinned foot geometry (left unshifted).
 */
export function computeSoleGroundOffset(modelRoot: THREE.Object3D): number {
  modelRoot.updateMatrixWorld(true)
  let lowest = Infinity
  const v = new THREE.Vector3()

  modelRoot.traverse((child) => {
    if (!(child instanceof THREE.SkinnedMesh) || !child.skeleton) return

    const footBones = new Set<number>()
    child.skeleton.bones.forEach((bone, i) => {
      if (FOOT_BONE_PATTERN.test(bone.name)) footBones.add(i)
    })
    if (footBones.size === 0) return

    const position = child.geometry.getAttribute('position')
    const skinIndex = child.geometry.getAttribute('skinIndex')
    const skinWeight = child.geometry.getAttribute('skinWeight')
    if (!position || !skinIndex || !skinWeight) return

    for (let i = 0; i < position.count; i++) {
      let footInfluence = 0
      for (let k = 0; k < 4; k++) {
        if (footBones.has(skinIndex.getComponent(i, k))) {
          footInfluence += skinWeight.getComponent(i, k)
        }
      }
      if (footInfluence < FOOT_INFLUENCE_THRESHOLD) continue
      v.fromBufferAttribute(position, i)
      child.applyBoneTransform(i, v) // bind-pose skinned position, mesh-local
      child.localToWorld(v) // → model-root space (root is at origin here)
      if (v.y < lowest) lowest = v.y
    }
  })

  return Number.isFinite(lowest) ? -lowest + FOOT_GROUND_CLEARANCE : 0
}

/** Tiny gap (m) kept between a settled corpse and the floor. */
const CORPSE_GROUND_CLEARANCE = 0.01

/**
 * Y offset (metres) to add to `model` so the body of its CURRENT animated pose
 * rests on the floor. Call once the death clip has clamped on its final frame.
 *
 * Unlike computeSoleGroundOffset this scans every skinned vertex, not just
 * soles: a fallen corpse touches down with its back or flank, and the monster
 * rigs have no foot/toe bones to key off. A monster whose lowest vertex is a
 * dangling appendage ends up hovering on its body — nudge it with
 * corpseGroundOffset in monsters.csv. Returns 0 with no skinned geometry.
 */
export function computeCorpseGroundOffset(model: THREE.Object3D): number {
  model.updateMatrixWorld(true)
  let lowest = Infinity
  const v = new THREE.Vector3()

  model.traverse((child) => {
    if (!(child instanceof THREE.SkinnedMesh) || !child.skeleton) return
    const position = child.geometry.getAttribute('position')
    if (!position) return

    for (let i = 0; i < position.count; i++) {
      v.fromBufferAttribute(position, i)
      child.applyBoneTransform(i, v) // skinned position in the current pose
      child.localToWorld(v)
      model.worldToLocal(v) // → model-local, independent of model.position
      if (v.y < lowest) lowest = v.y
    }
  })

  return Number.isFinite(lowest) ? -lowest + CORPSE_GROUND_CLEARANCE : 0
}

function findPrimarySkinnedMesh(
  root: THREE.Object3D
): THREE.SkinnedMesh | null {
  let bestMatch: THREE.SkinnedMesh | null = null

  root.traverse((child) => {
    if (!(child instanceof THREE.SkinnedMesh) || !child.skeleton) return
    if (
      !bestMatch ||
      child.skeleton.bones.length > bestMatch.skeleton.bones.length
    ) {
      bestMatch = child
    }
  })

  return bestMatch
}

function quaternionDistance(a: THREE.Quaternion, b: THREE.Quaternion): number {
  const direct = Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z, a.w - b.w)
  const negated = Math.hypot(a.x + b.x, a.y + b.y, a.z + b.z, a.w + b.w)
  return Math.min(direct, negated)
}

function roundForProfile(value: number): number {
  return Math.round(value * 1000) / 1000
}

function buildSkeletonProfileKey(skinnedMesh: THREE.SkinnedMesh): string {
  const sortedBones = [...skinnedMesh.skeleton.bones].sort((a, b) =>
    a.name.localeCompare(b.name)
  )
  return sortedBones
    .map((bone) =>
      [
        bone.name,
        roundForProfile(bone.position.x),
        roundForProfile(bone.position.y),
        roundForProfile(bone.position.z),
        roundForProfile(bone.quaternion.x),
        roundForProfile(bone.quaternion.y),
        roundForProfile(bone.quaternion.z),
        roundForProfile(bone.quaternion.w),
        roundForProfile(bone.scale.x),
        roundForProfile(bone.scale.y),
        roundForProfile(bone.scale.z),
      ].join(':')
    )
    .join('|')
}

function hasEquivalentSkeletonRestPose(
  targetSkinnedMesh: THREE.SkinnedMesh,
  sourceSkinnedMesh: THREE.SkinnedMesh
): boolean {
  const targetBones = targetSkinnedMesh.skeleton.bones.filter(
    (bone) => bone.name.length > 0
  )
  const sourceBoneByName = new Map(
    sourceSkinnedMesh.skeleton.bones
      .filter((bone) => bone.name.length > 0)
      .map((bone) => [bone.name, bone])
  )

  const commonBones = targetBones.filter((bone) =>
    sourceBoneByName.has(bone.name)
  )
  const coverage =
    commonBones.length / Math.max(targetBones.length, sourceBoneByName.size)
  if (coverage < 0.95) return false

  for (const targetBone of commonBones) {
    const sourceBone = sourceBoneByName.get(targetBone.name)
    if (!sourceBone) return false

    if (targetBone.position.distanceTo(sourceBone.position) > 0.001)
      return false
    if (targetBone.scale.distanceTo(sourceBone.scale) > 0.001) return false
    if (
      quaternionDistance(targetBone.quaternion, sourceBone.quaternion) > 0.001
    ) {
      return false
    }
  }

  return true
}

function normalizeRetargetedClipTrackNames(
  retargetedClip: THREE.AnimationClip,
  originalClipName: string
): THREE.AnimationClip {
  let renamedTrackFound = false
  const convertedTracks: THREE.KeyframeTrack[] = []

  for (const track of retargetedClip.tracks) {
    const match = RETARGET_TRACK_NAME_PATTERN.exec(track.name)
    if (!match) {
      convertedTracks.push(track)
      continue
    }

    const [, boneName, property] = match

    const renamedTrack = track.clone()
    renamedTrack.name = `${boneName}.${property}`
    renamedTrackFound = true
    convertedTracks.push(renamedTrack)
  }

  if (!renamedTrackFound) return retargetedClip

  return new THREE.AnimationClip(
    originalClipName,
    retargetedClip.duration,
    convertedTracks
  )
}

function buildBoneNameMap(
  targetSkinnedMesh: THREE.SkinnedMesh,
  sourceSkinnedMesh: THREE.SkinnedMesh
): Record<string, string> {
  const sourceBoneNames = new Set(
    sourceSkinnedMesh.skeleton.bones
      .map((bone) => bone.name)
      .filter((name) => name.length > 0)
  )
  const nameMap: Record<string, string> = {}

  for (const targetBone of targetSkinnedMesh.skeleton.bones) {
    if (!targetBone.name || !sourceBoneNames.has(targetBone.name)) continue
    nameMap[targetBone.name] = targetBone.name
  }

  return nameMap
}

function resolveHipBoneName(sourceSkinnedMesh: THREE.SkinnedMesh): string {
  const sourceBoneNames = new Set(
    sourceSkinnedMesh.skeleton.bones.map((bone) => bone.name)
  )
  return (
    HIP_BONE_CANDIDATES.find((boneName) => sourceBoneNames.has(boneName)) ??
    sourceSkinnedMesh.skeleton.bones[0]?.name ??
    'Hips'
  )
}

function getHipBoneRestY(
  skinnedMesh: THREE.SkinnedMesh,
  hipBoneName: string
): number | null {
  const hipBone = skinnedMesh.skeleton.bones.find((b) => b.name === hipBoneName)
  return hipBone ? hipBone.position.y : null
}

function correctHipHeightInClip(
  clip: THREE.AnimationClip,
  hipBoneName: string,
  yDelta: number
): void {
  const track = clip.tracks.find((t) => t.name === `${hipBoneName}.position`)
  if (!track) return

  // Position values are stored as [x, y, z, x, y, z, ...]
  const values = track.values
  for (let i = 1; i < values.length; i += 3) {
    values[i] += yDelta
  }
}

export async function retargetAnimationsForCharacterModel(
  targetScene: THREE.Object3D,
  retargetSourceScene: THREE.Object3D | null | undefined,
  clips: THREE.AnimationClip[]
): Promise<THREE.AnimationClip[]> {
  if (!ENABLE_RUNTIME_BONE_RETARGETING) return clips
  if (clips.length === 0 || !retargetSourceScene) return clips

  // Fast path: check the batch cache before the expensive cloning. Clones share
  // their source's geometry, so the two uuids identify the skeleton pair across
  // every clone of the same GLB — another model gets its own entry.
  const batchKey = [
    findPrimarySkinnedMesh(targetScene)?.geometry.uuid,
    findPrimarySkinnedMesh(retargetSourceScene)?.geometry.uuid,
    clips.map((c) => c.name).join(','),
  ].join('::')
  const cachedBatch = retargetBatchCache.get(batchKey)
  if (cachedBatch) return cachedBatch

  // Operate on clones only. Both target and source scenes can come from shared
  // loader instances, and retarget internals mutate skeleton transforms.
  const targetSceneClone = SkeletonUtils.clone(targetScene) as THREE.Object3D

  // `retargetSourceScene` comes from a shared loader cache. Retargeting mutates
  // skeleton state (`pose`, matrix updates), so work on a clone to avoid
  // leaking transforms back into female_knight.glb previews.
  const retargetSourceClone = SkeletonUtils.clone(
    retargetSourceScene
  ) as THREE.Object3D

  const targetSkinnedMesh = findPrimarySkinnedMesh(targetSceneClone)
  const sourceSkinnedMesh = findPrimarySkinnedMesh(retargetSourceClone)
  if (!targetSkinnedMesh || !sourceSkinnedMesh) return clips

  targetSkinnedMesh.skeleton.pose()
  sourceSkinnedMesh.skeleton.pose()
  targetSkinnedMesh.updateMatrixWorld(true)
  sourceSkinnedMesh.updateMatrixWorld(true)

  if (hasEquivalentSkeletonRestPose(targetSkinnedMesh, sourceSkinnedMesh)) {
    return clips
  }

  const boneNameMap = buildBoneNameMap(targetSkinnedMesh, sourceSkinnedMesh)
  if (Object.keys(boneNameMap).length === 0) return clips

  const targetProfileKey = buildSkeletonProfileKey(targetSkinnedMesh)
  const sourceProfileKey = buildSkeletonProfileKey(sourceSkinnedMesh)
  const hipBoneName = resolveHipBoneName(sourceSkinnedMesh)

  const targetHipY = getHipBoneRestY(targetSkinnedMesh, hipBoneName)
  const sourceHipY = getHipBoneRestY(sourceSkinnedMesh, hipBoneName)
  const hipYDelta =
    targetHipY !== null && sourceHipY !== null ? targetHipY - sourceHipY : 0

  const retargetedClips: THREE.AnimationClip[] = []
  for (const clip of clips) {
    const cacheKey = `${targetProfileKey}::${sourceProfileKey}::${clip.name}`
    const cachedClip = retargetedClipCache.get(cacheKey)
    if (cachedClip) {
      retargetedClips.push(cachedClip)
      continue
    }

    try {
      targetSkinnedMesh.skeleton.pose()
      targetSkinnedMesh.updateMatrixWorld(true)

      const retargetedClip = SkeletonUtils.retargetClip(
        targetSkinnedMesh,
        sourceSkinnedMesh,
        clip,
        {
          names: boneNameMap,
          hip: hipBoneName,
          preserveBoneMatrix: true,
          useTargetMatrix: false,
          useFirstFramePosition: false,
        }
      )
      const normalizedClip = normalizeRetargetedClipTrackNames(
        retargetedClip,
        clip.name
      )
      if (normalizedClip.tracks.length === 0) {
        retargetedClips.push(clip)
        continue
      }
      if (Math.abs(hipYDelta) > 0.001) {
        correctHipHeightInClip(normalizedClip, hipBoneName, hipYDelta)
      }
      retargetedClipCache.set(cacheKey, normalizedClip)
      retargetedClips.push(normalizedClip)
    } catch (error) {
      console.warn(`Failed to retarget animation clip "${clip.name}"`, error)
      retargetedClips.push(clip)
    }

    // Yield to the browser after each clip so the render loop keeps running
    await new Promise((r) => setTimeout(r, 0))
  }

  retargetBatchCache.set(batchKey, retargetedClips)
  return retargetedClips
}

export async function retargetOrderedCharacterAnimationsForModel(
  targetScene: THREE.Object3D,
  orderedSelections: OrderedAnimationSelection[],
  sourceScenes: RetargetSourceScenes
): Promise<THREE.AnimationClip[]> {
  if (!ENABLE_RUNTIME_BONE_RETARGETING) {
    return orderedSelections.map((selection) => selection.clip)
  }

  const bySource = {
    base: orderedSelections.filter((selection) => selection.source === 'base'),
    locomotion: orderedSelections.filter(
      (selection) => selection.source === 'locomotion'
    ),
    combat_melee: orderedSelections.filter(
      (selection) => selection.source === 'combat_melee'
    ),
  }

  // Each call yields per-clip internally so the browser can render frames
  const retargetedBase = await retargetAnimationsForCharacterModel(
    targetScene,
    sourceScenes.base,
    bySource.base.map((selection) => selection.clip)
  )

  const retargetedLocomotion = await retargetAnimationsForCharacterModel(
    targetScene,
    sourceScenes.locomotion,
    bySource.locomotion.map((selection) => selection.clip)
  )

  const retargetedCombatMelee = await retargetAnimationsForCharacterModel(
    targetScene,
    sourceScenes.combatMelee ?? sourceScenes.locomotion,
    bySource.combat_melee.map((selection) => selection.clip)
  )

  const retargetedBySource = {
    base: retargetedBase,
    locomotion: retargetedLocomotion,
    combat_melee: retargetedCombatMelee,
  }

  let baseIndex = 0
  let locomotionIndex = 0
  let combatMeleeIndex = 0

  return orderedSelections.map((selection) => {
    if (selection.source === 'base') {
      const clip = retargetedBySource.base[baseIndex]
      baseIndex += 1
      return clip ?? selection.clip
    }

    if (selection.source === 'locomotion') {
      const clip = retargetedBySource.locomotion[locomotionIndex]
      locomotionIndex += 1
      return clip ?? selection.clip
    }

    const clip = retargetedBySource.combat_melee[combatMeleeIndex]
    combatMeleeIndex += 1
    return clip ?? selection.clip
  })
}

export function selectOrderedCharacterAnimations(
  baseAnimations: THREE.AnimationClip[],
  locomotionAnimations: THREE.AnimationClip[],
  combatMeleeAnimations: THREE.AnimationClip[]
): OrderedAnimationSelection[] {
  const baseClipByName = new Map(
    baseAnimations.map((clip) => [clip.name, clip])
  )
  const locomotionClipByName = new Map(
    locomotionAnimations.map((clip) => [clip.name, clip])
  )
  const combatMeleeClipByName = new Map(
    combatMeleeAnimations.map((clip) => [clip.name, clip])
  )
  const firstBaseClip = baseAnimations[0]
  const firstLocomotionClip = locomotionAnimations[0]
  const firstCombatMeleeClip = combatMeleeAnimations[0]
  const orderedSelections: OrderedAnimationSelection[] = []

  for (const name of Object.values(AnimationName)) {
    const source = ANIMATION_SOURCE_BY_NAME[name]
    if (!source) {
      console.error(
        `No animation source mapping defined for "${name}"; update animation source map.`
      )
      return []
    }

    const selectedClip =
      source === 'locomotion'
        ? locomotionClipByName.get(name)
        : source === 'combat_melee'
          ? combatMeleeClipByName.get(name)
          : baseClipByName.get(name)

    let clip = selectedClip
    let fromFallback = false

    if (!clip) {
      const fallbackClip =
        source === 'locomotion'
          ? firstLocomotionClip
          : source === 'combat_melee'
            ? firstCombatMeleeClip
            : firstBaseClip

      if (fallbackClip) {
        clip = fallbackClip
        fromFallback = true
        console.warn(
          `Missing animation "${name}" in ${source}.glb; using first ${source} clip "${fallbackClip.name}" as fallback.`
        )
      }
    }

    if (!clip) {
      console.error(
        `Missing animation "${name}" in ${source}.glb and no fallback clip is available.`
      )
      return []
    }

    orderedSelections.push({
      name,
      clip,
      source,
      fromFallback,
    })
  }

  return orderedSelections
}

const sharedPackClipsByModel = new Map<string, Promise<THREE.AnimationClip[]>>()

/**
 * The named locomotion + melee clips retargeted onto a model rigged on the
 * character bone names but with its own bone offsets (monsters with
 * `sharedAnims`) — played unretargeted, combat_melee's clips stretch it.
 * Cached per model, so a crowd of one monster type retargets once.
 */
export function loadSharedPackClipsForModel(
  modelPath: string,
  targetScene: THREE.Object3D,
  clipNames: string[]
): Promise<THREE.AnimationClip[]> {
  const wanted = new Set(clipNames)
  const cacheKey = `${modelPath}::${[...wanted].sort().join(',')}`
  const cached = sharedPackClipsByModel.get(cacheKey)
  if (cached) return cached

  const clips = Promise.all([
    loadGLB(CHARACTER_ANIMATION_PACK_PATHS.locomotion),
    loadGLB(CHARACTER_ANIMATION_PACK_PATHS.combatMelee),
  ])
    .then((packs) =>
      Promise.all(
        packs.map((pack) =>
          retargetAnimationsForCharacterModel(
            targetScene,
            pack.scene,
            pack.animations.filter((clip) => wanted.has(clip.name))
          )
        )
      )
    )
    .then((packs) => packs.flat())
  sharedPackClipsByModel.set(cacheKey, clips)
  return clips
}

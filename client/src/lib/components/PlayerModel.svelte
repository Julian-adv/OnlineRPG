<script module lang="ts">
  import * as THREE from 'three'

  const HEALTH_BAR_WIDTH = 1.0
  const HEALTH_BAR_HEIGHT = 0.08

  // Shared across all PlayerModel instances — the fill geometry never changes.
  // Left-anchored via translate so the mesh only needs scale.x to grow/shrink.
  const healthBarFillGeometry = new THREE.PlaneGeometry(
    HEALTH_BAR_WIDTH,
    HEALTH_BAR_HEIGHT
  )
  healthBarFillGeometry.translate(HEALTH_BAR_WIDTH / 2, 0, 0)
</script>

<script lang="ts">
  import { T } from '@threlte/core'
  import TextLabel from './TextLabel.svelte'
  import type { Vector3 } from 'three'
  import type { GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js'
  import { onMount } from 'svelte'
  import { SvelteMap } from 'svelte/reactivity'
  import { get } from 'svelte/store'
  import { timeScale } from '../stores/timeStore'
  import {
    AnimationIndex,
    AnimationName,
    FishingAnimationName,
    OffhandAnimationName,
    TORCH_IDLE_CLIP_NAMES,
  } from '../types/animations'
  import {
    computeSoleGroundOffset,
    createCharacterModelRoot,
    getGltfAnimations,
    retargetOrderedCharacterAnimationsForModel,
    selectOrderedCharacterAnimations,
  } from '../utils/characterAnimationUtils'
  import {
    CHARACTER_ANIMATION_PACK_PATHS,
    getCharacterModelPath,
    getDefaultWeaponModel,
    getWeaponModelPath,
  } from '../utils/modelPaths'
  import { loadGLB } from '../utils/gltfCache'
  import { pickRandom } from '../utils/randomUtils'
  import { inventoryStore } from '../stores/inventoryStore'
  import { getItemDef } from '../data/itemDefs'
  import { torchLightEnabled } from '../stores/debugStore'
  import { localPlayerRightHand } from '../stores/playerHandRegistry'

  import type { CharacterClass, Gender } from '../network/networkTypes'
  import {
    type MovementMode,
    type PlayerStateName,
  } from '../utils/movementUtils'
  import { TorchFireParticles } from '../effects/torch-fire-particles'
  import ChatBubble from './ChatBubble.svelte'
  import DamageText from './DamageText.svelte'
  import type { PlayerDamageInfo, PlayerGoldInfo } from '../stores/gameStore'

  interface Props {
    position: Vector3
    name: string
    isCurrentPlayer: boolean
    playerState: PlayerStateName
    interactionAnim?: string
    interactOffsetY?: number
    attackCounter?: number
    speed: number
    rotation: number
    movementMode?: MovementMode
    camera: THREE.Camera | undefined
    chatBubble?: string
    characterClass: CharacterClass
    gender: Gender
    health: number
    maxHealth: number
    onAttackDuration?: (duration: number) => void
    onDyingFinished?: () => void
    onInteractionFinished?: () => void
    onPickupGrab?: () => void
    isLoading?: boolean
    lastDamageInfo?: PlayerDamageInfo
    lastRegenInfo?: PlayerDamageInfo
    lastGoldInfo?: PlayerGoldInfo
    torchOn?: boolean
    torchEffectsDisabled?: boolean
    /** Set for NPC remote players so canvas clicks can resolve this model
     *  back to its player id (read from userData by the input raycast). */
    npcPlayerId?: number
  }

  let {
    position,
    name,
    isCurrentPlayer,
    playerState,
    interactionAnim,
    interactOffsetY = 0,
    attackCounter,
    speed: _speed,
    rotation,
    movementMode,
    camera,
    chatBubble,
    characterClass,
    gender,
    health,
    maxHealth,
    onAttackDuration,
    onDyingFinished,
    onInteractionFinished,
    onPickupGrab,
    isLoading = $bindable(false),
    lastDamageInfo,
    lastRegenInfo,
    lastGoldInfo,
    torchOn = false,
    torchEffectsDisabled = false,
    npcPlayerId,
  }: Props = $props()

  const DEFAULT_IDLE_INDICES = [
    AnimationIndex.IDLE1,
    AnimationIndex.IDLE2,
    AnimationIndex.IDLE3,
    AnimationIndex.IDLE4,
    AnimationIndex.IDLE5,
  ]

  let nametagScale = $state(1)
  let nametagHeight = $state(2.7)
  let nametagGroup = $state<THREE.Group | undefined>(undefined)
  let chatBubbleInstance = $state<ChatBubble | null>(null)
  let animDebugInfo = $state('')

  // Floating damage text
  let damageTextRef = $state<ReturnType<typeof DamageText>>()

  // svelte-ignore state_referenced_locally
  let displayedHealth = $state(health)

  // Heals (and remote players) update the bar immediately; it never drops here.
  $effect(() => {
    if (!isCurrentPlayer || health >= displayedHealth) {
      displayedHealth = health
    }
  })

  // Damage drops the bar only when a new damage event arrives, keeping it in
  // sync with the floating damage text (emitted on the same delay). A fresh
  // lastDamageInfo object fires this once per hit; health is not a dependency,
  // so a server health update alone won't drop the bar early.
  $effect(() => {
    if (isCurrentPlayer && lastDamageInfo) {
      displayedHealth = lastDamageInfo.currentHealth ?? health
    }
  })

  let displayedHealthRatio = $derived(
    Math.max(0, Math.min(1, displayedHealth / (maxHealth || 1)))
  )

  // Load only the active character model + shared animation packs via shared cache.
  // This cache persists across Threlte Canvas lifecycles, so GLBs loaded in
  // character select don't re-download when entering the game scene.
  let activeGltfData = $state<GLTF | null>(null)
  let locomotionGltfData = $state<GLTF | null>(null)
  let combatMeleeGltfData = $state<GLTF | null>(null)
  let weaponGltfData = $state<GLTF | null>(null)

  // svelte-ignore state_referenced_locally
  const defaultWeaponModel = getDefaultWeaponModel(characterClass)
  // svelte-ignore state_referenced_locally
  const modelPath = getCharacterModelPath(characterClass, gender)
  const modelPromise = loadGLB(modelPath).then((g) => {
    activeGltfData = g
  })
  const locomotionPromise = loadGLB(
    CHARACTER_ANIMATION_PACK_PATHS.locomotion
  ).then((g) => {
    locomotionGltfData = g
  })
  const combatMeleePromise = loadGLB(
    CHARACTER_ANIMATION_PACK_PATHS.combatMelee
  ).then((g) => {
    combatMeleeGltfData = g
  })
  const weaponPromise = defaultWeaponModel
    ? loadGLB(getWeaponModelPath(defaultWeaponModel)).then((g) => {
        weaponGltfData = g
      })
    : Promise.resolve()
  const glbReady = Promise.all([
    modelPromise,
    locomotionPromise,
    combatMeleePromise,
    weaponPromise,
  ])

  // Animation system - following gpt-all-in-one.html approach
  let mixer = $state<THREE.AnimationMixer | null>(null)
  let currentAction = $state<THREE.AnimationAction | null>(null)
  let modelRoot = $state<THREE.Group | null>(null)
  let modelGroup = $state<THREE.Group | undefined>(undefined)

  let clonedScene: THREE.Object3D | null = null
  let validAnimations = $state<THREE.AnimationClip[]>([])
  let offhandClips = new SvelteMap<string, THREE.AnimationClip>()
  let socialClipsByName = new SvelteMap<string, THREE.AnimationClip>()
  let socialLoading = false
  let lastPlayerState = $state<PlayerStateName | undefined>(undefined)
  let lastAttackCounter = $state(0)
  let dyingFinishedNotified = $state(false)
  let interactionFinishedNotified = $state(false)
  let pickupGrabNotified = $state(false)
  let currentMovementAnimationIndex = $state<number | undefined>(undefined) // Locked animation for current movement
  let weaponAttached = $state(false)
  let weaponObject: THREE.Object3D | null = null
  const OVERLAP_BEFORE_END = 0.3 // Start next animation overlap 0.3 seconds before current ends
  const _nametagPos = new THREE.Vector3()

  function findPrimarySkinnedMesh(
    root: THREE.Object3D
  ): THREE.SkinnedMesh | undefined {
    let primarySkinnedMesh: THREE.SkinnedMesh | undefined
    root.traverse((obj) => {
      if (!(obj instanceof THREE.SkinnedMesh) || !obj.skeleton) return
      if (
        !primarySkinnedMesh ||
        obj.skeleton.bones.length > primarySkinnedMesh.skeleton.bones.length
      ) {
        primarySkinnedMesh = obj
      }
    })
    return primarySkinnedMesh
  }

  function findBoneByName(
    root: THREE.Object3D,
    name: string
  ): THREE.Bone | undefined {
    const primarySkinnedMesh = findPrimarySkinnedMesh(root)
    if (!primarySkinnedMesh) return undefined
    return primarySkinnedMesh.skeleton.bones.find((bone) => bone.name === name)
  }

  // The rod GLB is baked to the spear hand-socket (tip ~38° below
  // horizontal); lift it so it reads held up in every fishing pose.
  const FISHING_ROD_PITCH = Math.PI / 4

  function attachWeaponModel(
    gltfScene: THREE.Object3D,
    characterRoot: THREE.Object3D,
    itemDefId?: string | null
  ): boolean {
    const rightHandBone = findBoneByName(characterRoot, 'RightHand')
    if (!rightHandBone) {
      console.warn('Could not find right hand bone for weapon attachment')
      return false
    }

    weaponObject = gltfScene.clone()
    // Offset from wrist bone toward palm so weapon looks gripped
    weaponObject.position.set(0, 0.08, 0)
    if (itemDefId === 'fishing_rod') {
      weaponObject.rotation.x = FISHING_ROD_PITCH
    }
    rightHandBone.add(weaponObject)
    weaponAttached = true
    return true
  }

  function tryAttachWeapon(characterRoot: THREE.Object3D): boolean {
    if (!defaultWeaponModel || weaponAttached || !weaponGltfData) return false
    return attachWeaponModel(weaponGltfData.scene, characterRoot)
  }

  function detachWeapon() {
    if (weaponObject && weaponObject.parent) {
      weaponObject.parent.remove(weaponObject)
    }
    weaponObject = null
    weaponAttached = false
  }

  let offhandObject: THREE.Object3D | null = null
  let torchTipNode: THREE.Object3D | null = null
  const FALLBACK_TORCH_TIP_LOCAL_OFFSET = new THREE.Vector3(0.6, 0, 0)

  function attachOffhandModel(
    gltfScene: THREE.Object3D,
    characterRoot: THREE.Object3D
  ): boolean {
    const leftHandBone = findBoneByName(characterRoot, 'LeftHand')
    if (!leftHandBone) {
      console.warn('Could not find left hand bone for off-hand attachment')
      return false
    }

    offhandObject = gltfScene.clone()
    offhandObject.position.set(0, 0.08, 0)
    offhandObject.rotation.y = Math.PI
    leftHandBone.add(offhandObject)
    torchTipNode = offhandObject.getObjectByName('torch_tip') ?? null
    return true
  }

  function detachOffhand() {
    if (offhandObject && offhandObject.parent) {
      offhandObject.parent.remove(offhandObject)
    }
    offhandObject = null
    torchTipNode = null
  }

  const equippedMainHandItemId = $derived(
    isCurrentPlayer
      ? ($inventoryStore.equipped.main_hand?.item_def_id ?? null)
      : null
  )

  let attachedWeaponItemId: string | null = null
  let weaponAttachGeneration = 0

  $effect(() => {
    const itemDefId = equippedMainHandItemId
    // Read modelRoot so effect re-runs when model finishes loading
    const root = modelRoot
    if (!root || !clonedScene) return

    if (itemDefId === attachedWeaponItemId) return

    detachWeapon()
    attachedWeaponItemId = null

    if (!itemDefId) return

    const itemDef = getItemDef(itemDefId)
    if (!itemDef?.worldModel) return

    const gen = ++weaponAttachGeneration
    const weaponModelPath = getWeaponModelPath(itemDef.worldModel)
    loadGLB(weaponModelPath).then((gltf) => {
      if (gen !== weaponAttachGeneration || !clonedScene) return

      attachWeaponModel(gltf.scene, clonedScene, itemDefId)
      attachedWeaponItemId = itemDefId
    })
  })

  // Off-hand equip tracking. Local player uses inventory with a debug-toggle
  // fallback; remote players receive `torchOn` broadcast from the server.
  const equippedOffHandItemId = $derived(
    torchEffectsDisabled
      ? null
      : isCurrentPlayer
        ? ($inventoryStore.equipped.off_hand?.item_def_id ??
          ($torchLightEnabled ? 'torch' : null))
        : torchOn
          ? 'torch'
          : null
  )

  let attachedOffhandItemId: string | null = null
  let offhandAttachGeneration = 0

  $effect(() => {
    const itemDefId = equippedOffHandItemId
    const root = modelRoot
    if (!root || !clonedScene) return

    if (itemDefId === attachedOffhandItemId) return

    detachOffhand()
    detachTorchFire()
    attachedOffhandItemId = null
    if (mixer) playAnimationForState()

    if (!itemDefId) return

    const itemDef = getItemDef(itemDefId)
    if (!itemDef?.worldModel) return

    const gen = ++offhandAttachGeneration
    const offhandModelPath = getWeaponModelPath(itemDef.worldModel)
    loadGLB(offhandModelPath).then(async (gltf) => {
      if (gen !== offhandAttachGeneration || !clonedScene) return

      attachOffhandModel(gltf.scene, clonedScene)
      attachedOffhandItemId = itemDefId
      if (itemDefId === 'torch') {
        attachTorchFire()
        await loadOffhandAnimations()
        if (gen !== offhandAttachGeneration) return
        if (mixer) playAnimationForState()
      }
    })
  })

  // ── Torch fire particles ────────────────────────────────
  let torchFire: TorchFireParticles | null = null
  let torchFireGroup = $state<THREE.Group | null>(null)
  const _torchTipWorld = new THREE.Vector3()

  function attachTorchFire() {
    if (!torchFire) {
      torchFire = new TorchFireParticles()
      torchFireGroup = torchFire.group
    }
  }

  function detachTorchFire() {
    if (torchFire) {
      torchFire.dispose()
      torchFire = null
      torchFireGroup = null
    }
  }

  // Select movement animation based on movement mode
  function selectMovementAnimation(mode: MovementMode | undefined): number {
    if (mode === 'walk') return AnimationIndex.WALK
    if (mode === 'jog') return AnimationIndex.JOG
    if (mode === 'run') return AnimationIndex.RUN
    return AnimationIndex.JOG // Default fallback
  }

  async function loadSocialAnimations() {
    if (socialLoading || socialClipsByName.size > 0) return
    socialLoading = true
    try {
      // Interaction-state clips come from two packs; both land in the same
      // by-name map since interactionAnim is resolved purely by clip name.
      const [socialGltf, fishingGltf] = await Promise.all([
        loadGLB(CHARACTER_ANIMATION_PACK_PATHS.social),
        loadGLB(CHARACTER_ANIMATION_PACK_PATHS.fishing),
      ])
      for (const clip of getGltfAnimations(socialGltf)) {
        socialClipsByName.set(clip.name, clip)
      }
      for (const clip of getGltfAnimations(fishingGltf)) {
        socialClipsByName.set(clip.name, clip)
      }
    } finally {
      socialLoading = false
    }
    if (mixer && playerState === 'interact') playAnimationForState()
  }

  let offhandLoadPromise: Promise<void> | null = null

  function loadOffhandAnimations(): Promise<void> {
    if (offhandLoadPromise) return offhandLoadPromise
    offhandLoadPromise = (async () => {
      const offhandGltf = await loadGLB(CHARACTER_ANIMATION_PACK_PATHS.offhand)
      const rawClips = getGltfAnimations(offhandGltf)
      offhandClips.clear()
      for (const clip of rawClips) offhandClips.set(clip.name, clip)
    })()
    return offhandLoadPromise
  }

  function playAnimationForState() {
    // Check if mixer and animations are available
    if (!mixer || validAnimations.length === 0) return

    // Hide weapons during interact animations — except fishing, where the
    // held rod IS the point of the stance.
    const fishingInteraction =
      interactionAnim === FishingAnimationName.CAST ||
      interactionAnim === FishingAnimationName.IDLE
    if (weaponObject) {
      weaponObject.visible = playerState !== 'interact' || fishingInteraction
    }
    if (offhandObject) {
      offhandObject.visible = playerState !== 'interact'
    }
    if (torchFireGroup) {
      torchFireGroup.visible = playerState !== 'interact'
    }

    const hasTorch = attachedOffhandItemId === 'torch'
    const torchIdle = hasTorch
      ? pickRandom(
          TORCH_IDLE_CLIP_NAMES.map((name) => offhandClips.get(name)).filter(
            (c): c is THREE.AnimationClip => c !== undefined
          )
        )
      : undefined
    const torchWalk = hasTorch
      ? offhandClips.get(OffhandAnimationName.TORCH_WALK)
      : undefined
    const torchRun = hasTorch
      ? offhandClips.get(OffhandAnimationName.TORCH_RUN)
      : undefined
    let clip: THREE.AnimationClip | undefined
    if (playerState === 'idle') {
      currentMovementAnimationIndex = undefined
      clip =
        torchIdle ??
        pickRandom(DEFAULT_IDLE_INDICES.map((i) => validAnimations[i]))
    } else if (playerState === 'moving') {
      if (currentMovementAnimationIndex === undefined) {
        currentMovementAnimationIndex = selectMovementAnimation(movementMode)
      }
      const torchMoveClip = movementMode === 'walk' ? torchWalk : torchRun
      clip = torchMoveClip ?? validAnimations[currentMovementAnimationIndex]
    } else if (playerState === 'attack') {
      // Use slash1 animation
      currentMovementAnimationIndex = undefined
      // Find index for slash1 or fallback
      // Assuming AnimationIndex.SLASH1 exists and maps correctly
      clip = validAnimations[AnimationIndex.SLASH1]
    } else if (playerState === 'jump') {
      // One-shot feedback when slope is too steep to climb. After the clip
      // finishes, PlayerControl flips the state back to idle/moving and we
      // crossfade to the next animation naturally.
      currentMovementAnimationIndex = undefined
      clip = validAnimations[AnimationIndex.JUMP]
    } else if (playerState === 'dead') {
      currentMovementAnimationIndex = undefined
      dyingFinishedNotified = false
      clip = validAnimations[AnimationIndex.DYING]
    } else if (playerState === 'interact') {
      currentMovementAnimationIndex = undefined
      interactionFinishedNotified = false
      clip = interactionAnim
        ? socialClipsByName.get(interactionAnim)
        : undefined
      if (!clip) {
        loadSocialAnimations()
        return
      }
    } else {
      return // Unknown state
    }

    if (!clip) return

    const newAction = mixer.clipAction(clip)

    // The fishing idle is a stance held for the whole wait, not a one-shot
    // gesture like pickup — it loops until the fishing session ends.
    const playOnce =
      playerState !== 'moving' && interactionAnim !== FishingAnimationName.IDLE
    newAction.reset()
    newAction.loop = playOnce ? THREE.LoopOnce : THREE.LoopRepeat
    newAction.clampWhenFinished = playOnce
    newAction.paused = false

    // If there's a current action and it's different, crossfade to the new one
    if (currentAction && newAction !== currentAction) {
      const crossfadeDuration = 0.3 // 300ms crossfade

      // Use THREE.js built-in crossfade. warp=false: do NOT time-scale the
      // incoming clip to match the outgoing clip's length — that made a long
      // idle ("look around") whip past at several-times speed when blending in
      // from a short walk/attack clip.
      newAction.crossFadeFrom(currentAction, crossfadeDuration, false)
    }

    // Play the new action
    newAction.play()
    currentAction = newAction
  }

  async function setupRealAnimation() {
    const activeGltf = activeGltfData
    if (activeGltf && !mixer && !modelRoot) {
      console.log('Setting up real animation system')

      const { clonedScene: cloned, modelRoot: newModelRoot } =
        createCharacterModelRoot(activeGltf.scene)

      // Plant the soles on the floor. Measured once here in the bind pose
      // (deterministic, both feet down) instead of on the first animation
      // frame — that earlier approach sampled a randomly-picked idle clip, so
      // the lift differed every session and the character floated above flat
      // dungeon floors after a restart.
      cloned.position.y = computeSoleGroundOffset(newModelRoot)

      // For current player, weapon is reactively managed by $effect watching inventory.
      // For other players, attach based on character class.
      if (!isCurrentPlayer) tryAttachWeapon(cloned)

      const baseAnimations = getGltfAnimations(activeGltf)
      const locomotionAnimations = getGltfAnimations(locomotionGltfData)
      const combatMeleeAnimations = getGltfAnimations(combatMeleeGltfData)

      console.log(`Found ${baseAnimations.length} base animation clips`)
      console.log(
        `Found ${locomotionAnimations.length} locomotion animation clips`
      )
      console.log(
        `Found ${combatMeleeAnimations.length} combat melee animation clips`
      )

      // Collect all node names in the cloned model
      // eslint-disable-next-line svelte/prefer-svelte-reactivity
      const modelNodeNames = new Set()
      cloned.traverse((obj) => {
        if (obj.name) modelNodeNames.add(obj.name)
      })
      console.log(`Model has ${modelNodeNames.size} named nodes`)
      console.log('Model node names:', Array.from(modelNodeNames).slice(0, 10))

      const orderedSelections = selectOrderedCharacterAnimations(
        baseAnimations,
        locomotionAnimations,
        combatMeleeAnimations
      )
      validAnimations = await retargetOrderedCharacterAnimationsForModel(
        newModelRoot,
        orderedSelections,
        {
          base: activeGltf.scene,
          locomotion: locomotionGltfData?.scene,
          combatMelee: combatMeleeGltfData?.scene,
        }
      )

      for (const selection of orderedSelections) {
        if (selection.fromFallback) {
          console.log(
            `❌ Missing animation: ${selection.name} (using fallback)`
          )
        } else {
          const source =
            selection.source === 'locomotion'
              ? 'locomotion.glb'
              : selection.source === 'combat_melee'
                ? 'combat_melee.glb'
                : 'female_knight.glb'
          console.log(`✅ Found animation: ${selection.name} (${source})`)
        }

        if (selection.name === AnimationName.SLASH1 && onAttackDuration) {
          onAttackDuration(selection.clip.duration)
        }
      }

      console.log(`Found ${validAnimations.length} valid animations`)

      if (validAnimations.length > 0) {
        try {
          // Setup mixer
          mixer = new THREE.AnimationMixer(newModelRoot)

          // Play appropriate animation based on isMoving state
          playAnimationForState()
        } catch (error) {
          console.warn('Failed to start player animation clips', error)
          if (mixer) {
            mixer.stopAllAction()
            mixer = null
          }
          currentAction = null
          validAnimations = []
        }
      } else {
        console.warn('No suitable animations found with strict filtering')

        // Fallback: try to play any animation without filtering
        const fallbackAnimations =
          baseAnimations.length > 0
            ? baseAnimations
            : combatMeleeAnimations.length > 0
              ? combatMeleeAnimations
              : locomotionAnimations
        if (fallbackAnimations.length > 0) {
          console.log(
            'Trying fallback: playing first animation without filtering'
          )
          mixer = new THREE.AnimationMixer(newModelRoot)
          const clip = fallbackAnimations[0]
          console.log(
            `Playing fallback animation: ${clip.name}, duration: ${clip.duration}s`
          )

          currentAction = mixer.clipAction(clip)
          currentAction.reset()
          currentAction.loop = THREE.LoopRepeat
          currentAction.paused = false
          currentAction.play()
        } else {
          console.log('No animations available at all')
        }
      }

      clonedScene = cloned
      modelRoot = newModelRoot

      if (isCurrentPlayer) {
        const rightHand = findBoneByName(cloned, 'RightHand')
        if (rightHand) localPlayerRightHand.set(rightHand)
      }
    }
  }

  onMount(() => {
    // Wait for all GLTFs (character model + animation packs) to load
    isLoading = true
    glbReady
      .then(() => setupRealAnimation())
      .then(() => {
        isLoading = false
      })

    // Cleanup on unmount
    return () => {
      if (mixer) {
        mixer.stopAllAction()
        mixer = null
      }
      if (modelRoot) {
        modelRoot = null
      }
      clonedScene = null
      weaponAttached = false
      attachedWeaponItemId = null
      attachedOffhandItemId = null
      detachTorchFire()
      if (isCurrentPlayer) localPlayerRightHand.set(null)
    }
  })

  export function getNametagGroup() {
    return nametagGroup
  }

  export function getModelGroup() {
    return modelGroup
  }

  // Tag the model group so the click raycast can resolve NPC models
  // back to their player id.
  $effect(() => {
    if (modelGroup && npcPlayerId) {
      modelGroup.userData.npcPlayerId = npcPlayerId
    }
  })

  // Function to update mixer and animation state and nametag - called from GameScene gameLoop
  export function update(deltaTime: number) {
    // Sync Three.js group position directly from the Vector3 prop
    // (Svelte cannot track mutations on THREE.Vector3 objects)
    if (modelGroup) {
      const yOffset = playerState === 'interact' ? interactOffsetY : 0
      modelGroup.position.set(position.x, position.y + yOffset, position.z)
    }

    // Update nametag logic (formerly in useTask)
    if (camera && nametagGroup) {
      _nametagPos.set(position.x, position.y + 2.2, position.z)
      const dist = camera.position.distanceTo(_nametagPos)

      // Min distance (zoom in) = 5
      // Max distance (zoom out) = 20
      const minDist = 5
      const maxDist = 20

      // Scale: 0.5 to 1.0
      const minScale = 0.5
      const maxScale = 1.0

      // Height: 2.3 to 2.7
      const minHeight = 2.0
      const maxHeight = 2.5

      let t = (dist - minDist) / (maxDist - minDist)
      t = Math.max(0, Math.min(1, t)) // Clamp between 0 and 1

      nametagScale = minScale + t * (maxScale - minScale)
      nametagHeight = minHeight + t * (maxHeight - minHeight)

      // Update nametag group transform
      nametagGroup.position.set(
        position.x,
        position.y + nametagHeight,
        position.z
      )
      nametagGroup.scale.set(nametagScale, nametagScale, nametagScale)
      nametagGroup.quaternion.copy(camera.quaternion)
    }

    // Update floating damage texts
    if (camera) {
      damageTextRef?.update(
        deltaTime,
        position.x,
        position.y,
        position.z,
        camera
      )
    }

    if (chatBubbleInstance) {
      chatBubbleInstance.update()
    }

    if (!mixer) return

    // Update debug info for slow mode
    const currentTS = get(timeScale)
    if (currentTS < 1.0 && currentAction) {
      const time = currentAction.time.toFixed(2)
      const duration = currentAction.getClip().duration.toFixed(2)
      const animName = currentAction.getClip().name
      animDebugInfo = `[${animName}] ${time}s / ${duration}s`
    } else {
      animDebugInfo = ''
    }

    // Update mixer with provided deltaTime
    if (currentAction) {
      mixer.update(deltaTime)

      const clip = currentAction.getClip()
      if (clip && clip.duration > 0) {
        // Calculate remaining time (without modulo)
        const remainingTime = clip.duration - currentAction.time

        // Trigger next animation once when conditions are met (0.3 seconds remaining)
        if (remainingTime <= OVERLAP_BEFORE_END && playerState === 'idle') {
          playAnimationForState()
          return // Early return to prevent duplicate calls below
        }
      }
    }

    if (playerState !== 'dead') {
      dyingFinishedNotified = false
    } else if (
      isCurrentPlayer &&
      onDyingFinished &&
      !dyingFinishedNotified &&
      currentAction
    ) {
      const clip = currentAction.getClip()
      if (
        clip.name === AnimationName.DYING &&
        currentAction.time >= clip.duration - 0.001
      ) {
        dyingFinishedNotified = true
        onDyingFinished()
      }
    }

    if (playerState !== 'interact') {
      interactionFinishedNotified = false
      pickupGrabNotified = false
    } else if (
      currentAction &&
      interactionAnim &&
      // Pickup is the one interaction remote players end on their own: the
      // server broadcast is transient (no StopInteraction follows), so the
      // finish callback must fire for remotes too. Held poses (bench, forge)
      // keep waiting for their StopInteraction.
      (isCurrentPlayer || interactionAnim === 'pickup')
    ) {
      const clip = currentAction.getClip()
      if (clip.name === interactionAnim) {
        if (
          onPickupGrab &&
          !pickupGrabNotified &&
          interactionAnim === 'pickup' &&
          currentAction.time >= clip.duration * 0.35
        ) {
          pickupGrabNotified = true
          onPickupGrab()
        }
        if (
          onInteractionFinished &&
          !interactionFinishedNotified &&
          currentAction.time >= clip.duration - 0.001
        ) {
          interactionFinishedNotified = true
          onInteractionFinished()
        }
      }
    }

    // Update animation state
    if (validAnimations.length > 0) {
      // Only update animation if the player state has changed or attack counter increased
      // Note: idle transitions are handled above by OVERLAP_BEFORE_END logic
      if (
        lastPlayerState !== playerState ||
        (playerState === 'attack' && lastAttackCounter !== attackCounter)
      ) {
        lastPlayerState = playerState
        if (attackCounter !== undefined) lastAttackCounter = attackCounter
        playAnimationForState()
      }
    }

    // Update torch fire particles
    if (torchFire && offhandObject) {
      if (torchTipNode) {
        torchTipNode.getWorldPosition(_torchTipWorld)
      } else {
        offhandObject.updateWorldMatrix(true, false)
        _torchTipWorld.copy(FALLBACK_TORCH_TIP_LOCAL_OFFSET)
        offhandObject.localToWorld(_torchTipWorld)
      }
      torchFire.setTipPosition(_torchTipWorld)
      torchFire.update(deltaTime, camera)
    }
  }
</script>

<!-- Character Model -->
{#if modelRoot}
  <T.Group
    bind:ref={modelGroup}
    position={[position.x, position.y, position.z]}
    rotation={[0, rotation, 0]}
  >
    <!-- 3D Character Model with real animations -->
    <T is={modelRoot} />
  </T.Group>
{/if}

<!-- Torch fire particles (world space) -->
{#if torchFireGroup}
  <T is={torchFireGroup} />
{/if}

<!-- Name tag (separate from character to avoid rotation inheritance) -->
<T.Group bind:ref={nametagGroup}>
  <TextLabel
    text={name}
    fontSize={0.3}
    color={isCurrentPlayer ? '#4299e1' : '#ffffff'}
    outlineColor="#000000"
    outlineWidth={7}
    anchorX="center"
    anchorY="middle"
  />

  <!-- Health Bar -->
  {#if isCurrentPlayer}
    <T.Group position.y={-0.3}>
      <!-- Background (black) -->
      <T.Mesh>
        <T.PlaneGeometry args={[HEALTH_BAR_WIDTH, HEALTH_BAR_HEIGHT]} />
        <T.MeshBasicMaterial color="#000000" transparent opacity={0.5} />
      </T.Mesh>
      <!-- Foreground (red) -->
      <T.Mesh
        position.x={-HEALTH_BAR_WIDTH / 2}
        position.z={0.001}
        scale.x={Math.max(0.001, displayedHealthRatio)}
      >
        <T is={healthBarFillGeometry} />
        <T.MeshBasicMaterial color="#ff0000" />
      </T.Mesh>
    </T.Group>
  {/if}

  {#if animDebugInfo}
    <TextLabel
      text={animDebugInfo}
      fontSize={0.2}
      color="#ffff00"
      position={[0, 0.4, 0]}
      anchorX="center"
      anchorY="middle"
    />
  {/if}
</T.Group>

<!-- Chat bubble (appears above player when they send a message) -->
{#if chatBubble}
  <ChatBubble
    bind:this={chatBubbleInstance}
    {position}
    {camera}
    message={chatBubble}
  />
{/if}

<!-- Floating Damage Text -->
{#if isCurrentPlayer}
  <DamageText
    bind:this={damageTextRef}
    {lastDamageInfo}
    {lastRegenInfo}
    {lastGoldInfo}
    startYOffset={nametagHeight + 0.04}
  />
{/if}

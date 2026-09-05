<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import type { GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js'
  import { onDestroy, untrack } from 'svelte'
  import { AnimationIndex } from '../types/animations'
  import {
    createCharacterModelRoot,
    findBoneByName,
    getGltfAnimations,
    retargetOrderedCharacterAnimationsForModel,
    selectOrderedCharacterAnimations,
  } from '../utils/characterAnimationUtils'
  import {
    CHARACTER_ANIMATION_PACK_PATHS,
    getCharacterModelPath,
    getWeaponModelPath,
  } from '../utils/modelPaths'
  import { loadGLB } from '../utils/gltfCache'
  import { capeColorOf, getItemDef } from '../data/itemDefs'
  import { capeTextureUrl } from '../utils/networkUtils'
  import { isTorchItemDefId } from '../stores/inventoryStore'
  import {
    FALLBACK_TORCH_TIP_LOCAL_OFFSET,
    forearmLength,
    mainHandBoneFor,
    poseMainHandProp,
    poseOffHandProp,
    resolveTipNode,
  } from '../utils/handProps'
  import { TorchFireParticles } from '../effects/fire-particles'
  import {
    applyTorchFlickerWorld,
    TORCH_BASE_DECAY,
  } from '../utils/torchFlicker'
  import {
    attachCapeFit,
    capeCollarTuningFor,
    fitCapeToSkeleton,
    type CapeRig,
  } from '../effects/cape-rig'
  import type {
    CharacterClass,
    Gender,
    VisibleEquipment,
  } from '../network/networkTypes'

  interface Props {
    positionX: number
    positionY: number
    positionZ: number
    rotationY?: number
    selected: boolean
    characterClass: CharacterClass
    gender?: Gender
    equipment?: VisibleEquipment
    /** Needed to billboard the torch flame's particles. */
    camera?: THREE.Camera
  }

  let {
    positionX,
    positionY,
    positionZ,
    rotationY = 0,
    selected,
    characterClass,
    gender,
    equipment,
    camera,
  }: Props = $props()

  // Load via shared cache so GLBs persist across Canvas lifecycles
  let characterGltfData = $state<GLTF | null>(null)
  let locomotionGltfData = $state<GLTF | null>(null)
  let combatMeleeGltfData = $state<GLTF | null>(null)

  const modelPath = $derived(getCharacterModelPath(characterClass, gender))

  $effect(() => {
    loadGLB(modelPath).then((g) => {
      characterGltfData = g
    })
  })
  loadGLB(CHARACTER_ANIMATION_PACK_PATHS.locomotion).then((g) => {
    locomotionGltfData = g
  })
  loadGLB(CHARACTER_ANIMATION_PACK_PATHS.combatMelee).then((g) => {
    combatMeleeGltfData = g
  })

  let mixer = $state<THREE.AnimationMixer | null>(null)
  let currentAction = $state<THREE.AnimationAction | null>(null)
  let modelRoot = $state<THREE.Group | null>(null)
  let clonedScene: THREE.Object3D | null = null
  let footBones: THREE.Bone[] = []
  let validAnimations = $state<THREE.AnimationClip[]>([])
  let setupDone = $state(false)

  const OVERLAP_BEFORE_END = 0.3

  let gltfReady = $derived(
    !!characterGltfData && !!locomotionGltfData && !!combatMeleeGltfData
  )

  function playIdleAnimation() {
    if (!mixer || validAnimations.length === 0) return

    const idleIndices = [
      AnimationIndex.IDLE1,
      AnimationIndex.IDLE2,
      AnimationIndex.IDLE3,
      AnimationIndex.IDLE4,
      AnimationIndex.IDLE5,
    ]
    const idleIndex =
      idleIndices[Math.floor(Math.random() * idleIndices.length)]
    const clip = validAnimations[idleIndex]
    if (!clip) return

    const newAction = mixer.clipAction(clip)
    newAction.reset()
    newAction.loop = THREE.LoopOnce
    newAction.clampWhenFinished = true
    newAction.paused = !selected

    if (currentAction && newAction !== currentAction) {
      newAction.crossFadeFrom(currentAction, 0.3, true)
    }

    newAction.play()
    currentAction = newAction
  }

  let capeRig: CapeRig | null = null
  let heldProps: THREE.Object3D[] = []
  // Bumped on detach so a GLB that lands afterwards doesn't attach to a
  // discarded rig.
  let equipGeneration = 0

  let torchFire: TorchFireParticles | null = null
  let torchFireGroup = $state<THREE.Group | null>(null)
  let torchLight = $state<THREE.PointLight | null>(null)
  let torchTipNode: THREE.Object3D | null = null
  let torchFlickerTime = 0
  const _torchTipWorld = new THREE.Vector3()

  /** The select scene is lit far more dimly than the night world the torch
   *  constants are tuned for, and the tip sits centimetres from the wearer —
   *  at full strength the flame would blow the character out. */
  const TORCH_LIGHT_INTENSITY_SCALE = 0.03
  const TORCH_LIGHT_DISTANCE = 6

  function attachHeldItem(
    characterRoot: THREE.Object3D,
    itemDefId: string | null | undefined,
    mainHand: boolean
  ): void {
    if (!itemDefId) return
    const worldModel = getItemDef(itemDefId)?.worldModel
    if (!worldModel) return

    const gen = equipGeneration
    // A bow is held in the left hand, so the slot no longer fixes the bone.
    const boneName = mainHand ? mainHandBoneFor(itemDefId) : 'LeftHand'
    loadGLB(getWeaponModelPath(worldModel)).then((gltf) => {
      const bone = findBoneByName(characterRoot, boneName)
      if (gen !== equipGeneration || !bone) return

      const prop = gltf.scene.clone()
      if (mainHand) {
        poseMainHandProp(
          prop,
          itemDefId,
          boneName === 'LeftHand'
            ? forearmLength(characterRoot, boneName, `${modelPath}:${boneName}`)
            : undefined
        )
      } else poseOffHandProp(prop)
      bone.add(prop)
      heldProps.push(prop)

      if (isTorchItemDefId(itemDefId)) lightTorch(prop)
    })
  }

  /** The flame and its glow live in world space, following the tip node that
   *  rides the hand bone. */
  function lightTorch(torch: THREE.Object3D): void {
    torchTipNode = resolveTipNode(
      torch,
      'torch_tip',
      FALLBACK_TORCH_TIP_LOCAL_OFFSET
    )
    torchFire = new TorchFireParticles()
    torchFireGroup = torchFire.group

    // Starts dark; the first flicker frame places it and sets its intensity.
    torchLight = new THREE.PointLight(
      '#ffcc66',
      0,
      TORCH_LIGHT_DISTANCE,
      TORCH_BASE_DECAY
    )
  }

  function extinguishTorch(): void {
    torchFire?.dispose()
    torchFire = null
    torchFireGroup = null
    torchLight = null
    torchTipNode = null
  }

  function attachEquipment(
    characterRoot: THREE.Object3D,
    worn: VisibleEquipment | undefined
  ): void {
    attachHeldItem(characterRoot, worn?.main_hand, true)
    attachHeldItem(characterRoot, worn?.off_hand, false)

    const capeColor = capeColorOf(worn?.back, worn?.back_color)
    if (!capeColor) return

    const fit = fitCapeToSkeleton(
      characterRoot,
      capeCollarTuningFor(modelPath),
      modelPath
    )
    if (fit) {
      capeRig = attachCapeFit(fit, {
        color: capeColor,
        texture: capeTextureUrl(worn?.back_texture),
      })
    }
  }

  function detachEquipment(): void {
    equipGeneration++
    capeRig?.dispose()
    capeRig = null
    extinguishTorch()
    for (const prop of heldProps) prop.removeFromParent()
    heldProps = []
  }

  // Returning from the game refreshes the list without remounting the slot;
  // keyed on the visible fields so an unchanged list is a no-op.
  const wornKey = $derived(
    [
      equipment?.main_hand,
      equipment?.off_hand,
      equipment?.back,
      equipment?.back_color,
      equipment?.back_texture,
    ].join('|')
  )
  $effect(() => {
    void wornKey
    if (!modelRoot || !clonedScene) return
    attachEquipment(
      clonedScene,
      untrack(() => equipment)
    )
    return detachEquipment
  })

  // --- Exported interface for parent game loop ---

  export function isGltfReady(): boolean {
    return gltfReady
  }

  export function isSetUp(): boolean {
    return setupDone
  }

  export function setup(): void {
    if (
      setupDone ||
      !characterGltfData ||
      !locomotionGltfData ||
      !combatMeleeGltfData
    )
      return
    setupDone = true

    const sourceScene = characterGltfData.scene
    const { clonedScene: newClonedScene, modelRoot: newModelRoot } =
      createCharacterModelRoot(sourceScene)

    const orderedAnims = selectOrderedCharacterAnimations(
      getGltfAnimations(characterGltfData),
      getGltfAnimations(locomotionGltfData),
      getGltfAnimations(combatMeleeGltfData)
    )

    retargetOrderedCharacterAnimationsForModel(newModelRoot, orderedAnims, {
      base: sourceScene,
      locomotion: locomotionGltfData.scene,
      combatMelee: combatMeleeGltfData.scene,
    }).then((clips) => {
      validAnimations = clips

      if (validAnimations.length > 0) {
        try {
          mixer = new THREE.AnimationMixer(newModelRoot)
          playIdleAnimation()
        } catch (error) {
          console.warn('Failed to start preview animation clips', error)
          if (mixer) {
            mixer.stopAllAction()
            mixer = null
          }
          currentAction = null
          validAnimations = []
        }
      }

      // Set modelRoot only after animation is playing to avoid T-pose flash
      clonedScene = newClonedScene
      footBones = []
      newModelRoot.traverse((child) => {
        if (child instanceof THREE.Bone && /foot|toe/i.test(child.name)) {
          footBones.push(child)
        }
      })
      modelRoot = newModelRoot
    })
  }

  function updateTorch(delta: number): void {
    if (!torchFire || !torchTipNode) return

    torchTipNode.getWorldPosition(_torchTipWorld)
    torchFire.setOrigin(_torchTipWorld)
    torchFire.update(delta, camera)

    if (torchLight) {
      torchFlickerTime = applyTorchFlickerWorld(
        torchLight,
        torchFlickerTime,
        delta,
        _torchTipWorld.x,
        _torchTipWorld.y,
        _torchTipWorld.z,
        TORCH_LIGHT_INTENSITY_SCALE
      )
    }
  }

  const _footVec = new THREE.Vector3()

  export function update(delta: number): void {
    if (selected && mixer && currentAction) {
      mixer.update(delta)

      if (clonedScene && modelRoot && footBones.length > 0) {
        clonedScene.position.y = 0
        modelRoot.updateMatrixWorld(true)

        const groupWorldY = modelRoot.parent
          ? modelRoot.parent.getWorldPosition(_footVec).y
          : positionY
        let lowestFootY = Infinity
        for (const bone of footBones) {
          bone.getWorldPosition(_footVec)
          const localY = _footVec.y - groupWorldY
          if (localY < lowestFootY) lowestFootY = localY
        }
        clonedScene.position.y = -lowestFootY
      }

      const clip = currentAction.getClip()
      if (clip && clip.duration > 0) {
        const remainingTime = clip.duration - currentAction.time
        if (remainingTime <= OVERLAP_BEFORE_END) {
          playIdleAnimation()
        }
      }
    }

    // After the mixer, so cloth and flame ride the pose this frame draws. Both
    // step for every slot, not just the selected one: the cloth starts in its
    // bind pose and needs frames to fall into a hang, and an unlit torch on an
    // unselected character reads as a bug.
    capeRig?.update(delta, null)
    updateTorch(delta)
  }

  export function dispose(): void {
    if (mixer) {
      mixer.stopAllAction()
      mixer = null
    }
    currentAction = null
    clonedScene = null
    footBones = []
    modelRoot = null
    validAnimations = []
    setupDone = false
  }

  // Pause/resume on selection change
  $effect(() => {
    if (!mixer || !currentAction) return

    if (selected) {
      currentAction.paused = false
      return
    }

    currentAction.paused = true
    currentAction.time = 0
    mixer.setTime(0)
  })

  onDestroy(() => {
    dispose()
  })
</script>

{#if modelRoot}
  <T.Group position={[positionX, positionY, positionZ]} rotation.y={rotationY}>
    <T is={modelRoot} />
  </T.Group>
{/if}

{#if torchFireGroup}
  <T is={torchFireGroup} />
{/if}
{#if torchLight}
  <T is={torchLight} />
{/if}

<script lang="ts">
  import { T, useLoader } from '@threlte/core'
  import TextLabel from './TextLabel.svelte'
  import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
  import * as SkeletonUtils from 'three/examples/jsm/utils/SkeletonUtils.js'
  import * as THREE from 'three'
  import { get } from 'svelte/store'
  import { untrack } from 'svelte'
  import { timeScale } from '../stores/timeStore'
  import DamageText from './DamageText.svelte'

  import type { MonsterData } from '../types/Monster'
  import { getMonsterDef } from '../data/monsterDefs'
  import { getItemDef } from '../data/itemDefs'
  import {
    computeCorpseGroundOffset,
    loadSharedPackClipsForModel,
  } from '../utils/characterAnimationUtils'
  import { billboardScale } from '../utils/billboardScale'

  interface Props {
    position: { x: number; y: number; z: number }
    rotation: number
    monsterState: MonsterData['state']
    attackCounter?: number
    id: string
    type: string
    lastDamageInfo?: MonsterData['lastDamageInfo']
    droppedWeaponItemDefId?: string
    onHitFinished?: () => void
  }

  let {
    position,
    rotation,
    monsterState,
    attackCounter,
    id,
    type,
    lastDamageInfo,
    droppedWeaponItemDefId,
    onHitFinished,
  }: Props = $props()

  const def = $derived(getMonsterDef(type))

  // Monster type is fixed for the component's lifetime, so the model and any
  // hand weapon are resolved once at init from the initial type, not reactively.
  const initialDef = untrack(() => getMonsterDef(type))
  const initialModel = initialDef?.model ?? 'monsters/scp939.glb'
  const initialScale = initialDef?.scale ?? 1
  const isBoss = initialDef?.boss === true
  const gltf = useLoader(GLTFLoader).load(`/models/${initialModel}`)

  // Monsters rigged on the character skeleton borrow the player's animation
  // packs, which every client already has cached, instead of shipping clips.
  let sharedClips: THREE.AnimationClip[] = []

  // Every anim* field of the def, so only the clips this monster plays are
  // retargeted — the packs carry three times as many.
  const usedClipNames = Object.entries(initialDef ?? {})
    .filter(([key, value]) => key.startsWith('anim') && !!value)
    .map(([, value]) => value as string)

  function findClip(name: string): THREE.AnimationClip | undefined {
    return (
      $gltf?.animations.find((c) => c.name === name) ??
      sharedClips.find((c) => c.name === name)
    )
  }

  // Optional hand weapon, attached to a skeleton bone.
  const initialWeapon = initialDef?.weapon
  const initialWeaponBone = initialDef?.weaponBone
  const initialWeaponModel = initialWeapon
    ? (getItemDef(initialWeapon)?.worldModel ?? initialWeapon)
    : undefined
  const weaponGltf = initialWeaponModel
    ? useLoader(GLTFLoader).load(`/models/${initialWeaponModel}`)
    : undefined

  // Weapon grip transform relative to the attach bone, tuned by eye.
  const WEAPON_OFFSET = new THREE.Vector3(0, 0, 0)
  const WEAPON_ROTATION = new THREE.Euler(0, 0, 0)
  const WEAPON_SCALE = 1
  let weaponAttached = false
  let weaponObject: THREE.Object3D | undefined

  let mixer = $state<THREE.AnimationMixer | undefined>(undefined)
  let currentAction = $state<THREE.AnimationAction | undefined>(undefined)
  let model: THREE.Group | undefined = $state(undefined)
  let group = $state<THREE.Group>()
  let nametagGroup = $state<THREE.Group | undefined>(undefined)
  let nametagHeight = $state(2.5)
  let animDebugInfo = $state('')
  let isDeadAnimationFinished = $state(false)
  let isAttackAnimationFinished = $state(true)
  let lastMonsterState = $state<MonsterData['state'] | undefined>(undefined)
  let lastDeadAnimFinished = $state(false)
  let lastAttackAnimFinished = $state(true)
  let lastAttackCounter = $state<number | undefined>(undefined)
  let damageTextRef = $state<ReturnType<typeof DamageText>>()
  let lastAppliedOpacity = 1
  let materialsCloned = false
  let deadGroundApplied = false
  let corpseTimer = 0
  const CORPSE_FADE_START = 25
  const CORPSE_FADE_DURATION = 5

  function cloneMaterials() {
    if (materialsCloned || !model) return
    materialsCloned = true
    model.traverse((child) => {
      if ((child as THREE.Mesh).isMesh) {
        const mesh = child as THREE.Mesh
        if (Array.isArray(mesh.material)) {
          mesh.material = mesh.material.map((m) => m.clone())
        } else {
          mesh.material = mesh.material.clone()
        }
      }
    })
  }

  function applyOpacity(opacity: number) {
    if (!model || opacity === lastAppliedOpacity) return
    cloneMaterials()
    lastAppliedOpacity = opacity
    model.traverse((child) => {
      if ((child as THREE.Mesh).isMesh) {
        const mesh = child as THREE.Mesh
        const materials = Array.isArray(mesh.material)
          ? mesh.material
          : [mesh.material]
        for (const mat of materials) {
          mat.transparent = true
          mat.opacity = opacity
        }
        mesh.castShadow = opacity >= 0.25
      }
    })
  }

  function playAnimation(forceRestart = false) {
    if (!mixer || !$gltf) return

    // Monsters on the shared packs have no hit reaction: keep what is playing
    // and end the flinch right away.
    if (monsterState === 'hit' && !def?.animHit) {
      onHitFinished?.()
      return
    }

    let clipName = def?.animIdle ?? 'Idle'
    if (monsterState === 'walk') clipName = def?.animWalk ?? 'Walk'
    if (monsterState === 'run') clipName = def?.animRun ?? 'Run'
    if (monsterState === 'attack') {
      clipName = isAttackAnimationFinished
        ? (def?.animAttackIdle ?? def?.animIdle ?? 'Idle')
        : (def?.animAttack ?? 'Attack')
    }
    if (monsterState === 'hit') clipName = def?.animHit ?? 'Hit'
    if (monsterState === 'dead') {
      clipName = isDeadAnimationFinished
        ? (def?.animDead ?? 'Dead')
        : (def?.animDie ?? 'Die')
    }

    const clip = findClip(clipName)

    if (clip) {
      const newAction = mixer.clipAction(clip)
      if (newAction !== currentAction || forceRestart) {
        const fadeDuration =
          monsterState === 'hit'
            ? 0.03
            : monsterState === 'dead'
              ? (def?.deathFadeSeconds ?? 0.2)
              : 0.2

        if (currentAction && newAction !== currentAction) {
          currentAction.fadeOut(fadeDuration)
        }

        if (monsterState === 'dead') {
          if (clipName === (def?.animDie ?? 'Die')) {
            newAction.setLoop(THREE.LoopOnce, 1)
            newAction.clampWhenFinished = true
          } else {
            // Post-death pose clip should loop / stay idle
            newAction.setLoop(THREE.LoopRepeat, Infinity)
            newAction.clampWhenFinished = false
          }
        } else if (monsterState === 'hit') {
          newAction.setLoop(THREE.LoopOnce, 1)
          newAction.clampWhenFinished = true
        } else if (
          monsterState === 'attack' &&
          clipName === (def?.animAttack ?? 'Attack')
        ) {
          newAction.setLoop(THREE.LoopOnce, 1)
          newAction.clampWhenFinished = true
        } else {
          newAction.setLoop(THREE.LoopRepeat, Infinity)
          newAction.clampWhenFinished = false
          isDeadAnimationFinished = false
        }

        newAction.reset().fadeIn(fadeDuration).play()

        currentAction = newAction
      }
    } else {
      console.warn(
        `Animation ${clipName} not found used for state ${monsterState}`
      )
      if (monsterState === 'hit') {
        onHitFinished?.()
      }
      const firstClip = $gltf.animations[0] ?? sharedClips[0]
      if (!currentAction && firstClip) {
        const newAction = mixer.clipAction(firstClip)
        newAction.play()
        currentAction = newAction
      }
    }
  }

  export function update(deltaTime: number, camera?: THREE.Camera) {
    // 0. Sync Three.js group position imperatively so the refraction render
    //    (which runs during the game loop, before Svelte's reactive updates)
    //    sees the monster at its current position.
    if (group) {
      group.position.set(position.x, position.y, position.z)
      group.rotation.y = rotation
    }

    // 1. Sync animation with state
    if (monsterState !== 'attack') {
      isAttackAnimationFinished = true
    }
    if (
      lastAttackCounter !== attackCounter ||
      lastMonsterState !== monsterState ||
      lastDeadAnimFinished !== isDeadAnimationFinished ||
      lastAttackAnimFinished !== isAttackAnimationFinished
    ) {
      const attackCounterChanged = lastAttackCounter !== attackCounter
      if (attackCounterChanged && monsterState === 'attack') {
        isAttackAnimationFinished = false
      }
      lastAttackCounter = attackCounter
      lastMonsterState = monsterState
      lastDeadAnimFinished = isDeadAnimationFinished
      lastAttackAnimFinished = isAttackAnimationFinished
      playAnimation(attackCounterChanged && monsterState === 'attack')
    }

    // 2. Update damage texts
    if (camera) {
      damageTextRef?.update(
        deltaTime,
        position.x,
        position.y,
        position.z,
        camera
      )
    }

    // 3. Corpse fade
    if (monsterState === 'dead') {
      corpseTimer += deltaTime
      if (corpseTimer >= CORPSE_FADE_START) {
        const fadeProgress =
          (corpseTimer - CORPSE_FADE_START) / CORPSE_FADE_DURATION
        applyOpacity(Math.max(0, 1 - fadeProgress))
      }
    } else {
      corpseTimer = 0
    }

    // 4. Update mixer
    if (mixer) {
      mixer.update(deltaTime)

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
    }

    // Update nametag to face camera
    if (camera && nametagGroup) {
      nametagGroup.position.set(
        position.x,
        position.y + nametagHeight,
        position.z
      )
      const s = billboardScale(
        camera.position.distanceTo(nametagGroup.position)
      )
      nametagGroup.scale.set(s, s, s)
      nametagGroup.quaternion.copy(camera.quaternion)
    }
  }

  $effect(() => {
    if ($gltf) {
      // Clone the model for this instance
      if (!model) {
        const clonedScene = SkeletonUtils.clone($gltf.scene) as THREE.Group

        // Enable shadows on all meshes
        clonedScene.traverse((child) => {
          if ((child as THREE.Mesh).isMesh) {
            child.castShadow = true
            child.receiveShadow = true
            // Add user data to identify as monster part
            child.userData.monsterId = id
          }
        })

        if (isBoss) {
          // Hang the nameplate just above the scaled bind-pose head.
          const modelTop = new THREE.Box3().setFromObject(clonedScene).max.y
          nametagHeight = modelTop * initialScale + 0.4
        }

        model = clonedScene
        // Setup mixer on the cloned scene
        mixer = new THREE.AnimationMixer(clonedScene)

        mixer.addEventListener('finished', (e) => {
          const finishedClipName = e.action.getClip().name
          if (finishedClipName === (def?.animHit ?? 'Hit')) {
            onHitFinished?.()
          }
          if (finishedClipName === (def?.animAttack ?? 'Attack')) {
            isAttackAnimationFinished = true
          }
          if (finishedClipName === (def?.animDie ?? 'Die')) {
            isDeadAnimationFinished = true
            // The death clip clamps with the body still raised, so settle the
            // corpse onto the ground.
            if (model && !deadGroundApplied) {
              deadGroundApplied = true
              // corpseGroundOffset is authored in world metres; de-scale it
              // since model.position lives inside the scaled group.
              model.position.y +=
                computeCorpseGroundOffset(model) +
                (def?.corpseGroundOffset ?? 0) / initialScale
            }
          }
        })

        if (initialDef?.sharedAnims) {
          loadSharedPackClipsForModel(
            initialModel,
            $gltf.scene,
            usedClipNames
          ).then((clips) => {
            sharedClips = clips
            playAnimation()
          })
        } else {
          playAnimation()
        }
      }
    }
  })

  // Attach the hand weapon once both the model and weapon GLB are ready, and
  // detach it again if the monster dies and drops the weapon to the ground.
  $effect(() => {
    // The held weapon dropped on death — detach it if it was attached.
    if (monsterState === 'dead' && droppedWeaponItemDefId) {
      if (weaponObject) {
        weaponObject.removeFromParent()
        weaponObject = undefined
        weaponAttached = false
      }
      return
    }

    if (
      weaponAttached ||
      !model ||
      !initialWeapon ||
      !initialWeaponBone ||
      !weaponGltf ||
      !$weaponGltf
    )
      return

    let bone: THREE.Object3D | undefined
    model.traverse((o) => {
      if (o.name === initialWeaponBone) bone = o
    })
    if (!bone) {
      console.warn(`Weapon bone ${initialWeaponBone} not found on ${type}`)
      weaponAttached = true
      return
    }

    weaponObject = $weaponGltf.scene.clone(true)
    weaponObject.position.copy(WEAPON_OFFSET)
    weaponObject.rotation.copy(WEAPON_ROTATION)
    weaponObject.scale.setScalar(WEAPON_SCALE)
    weaponObject.traverse((child) => {
      if ((child as THREE.Mesh).isMesh) {
        const mesh = child as THREE.Mesh
        // Clone materials so corpse-fade opacity is per-instance.
        mesh.material = Array.isArray(mesh.material)
          ? mesh.material.map((m) => m.clone())
          : mesh.material.clone()
        mesh.castShadow = true
        mesh.receiveShadow = true
        // Clicking the weapon should still target the monster.
        child.userData.monsterId = id
      }
    })
    bone.add(weaponObject)
    weaponAttached = true
  })

  // Export the model group for raycasting from parent
  export function getMeshGroup() {
    return group
  }

  export function getNametagGroup() {
    return nametagGroup
  }
</script>

{#if model}
  <T.Group
    bind:ref={group}
    position={[position.x, position.y, position.z]}
    rotation={[0, rotation, 0]}
    scale={[initialScale, initialScale, initialScale]}
  >
    <T is={model} castShadow receiveShadow />
  </T.Group>
{/if}

<!-- Name tag / Debug info -->
<T.Group bind:ref={nametagGroup}>
  {#if isBoss && monsterState !== 'dead'}
    <TextLabel
      text={initialDef?.name ?? type}
      fontSize={0.3}
      color="#ffd166"
      outlineColor="#422d00"
      outlineWidth={5}
      position={[0, 0, 0]}
      anchorX="center"
      anchorY="bottom"
    />
  {/if}
  {#if animDebugInfo}
    <TextLabel
      text={id}
      fontSize={0.2}
      color="#ffffff"
      position={[0, 0.3, 0]}
      anchorX="center"
      anchorY="middle"
    />
    <TextLabel
      text={animDebugInfo}
      fontSize={0.2}
      color="#ffff00"
      position={[0, 0.6, 0]}
      anchorX="center"
      anchorY="middle"
    />
  {/if}
</T.Group>

<!-- Floating Damage Text; bosses spawn it above the nameplate so the number
     isn't drawn behind the name (both are transparent billboards). -->
<DamageText
  bind:this={damageTextRef}
  {lastDamageInfo}
  startYOffset={isBoss ? nametagHeight + 0.3 : 1.8 * initialScale}
/>

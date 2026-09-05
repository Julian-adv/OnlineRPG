<script lang="ts">
  // Visual tracer: hits track the target; misses keep their launch bearing.
  import { T, useTask } from '@threlte/core'
  import { Group, Quaternion, Vector3 } from 'three'
  import type { ArrowShot } from '../stores/arrowStore'
  import { landArrow } from '../stores/arrowStore'
  import { unwrapWorldXNear } from '../terrain/world-wrap'

  interface Props {
    shot: ArrowShot
    playerId: number
    /** The arrow model, loaded once by the caller and shared by every arrow. */
    model: Group | undefined
    /** Where the target is now, or null once it is dead or gone. */
    targetOf: () => { x: number; y: number; z: number } | null
  }

  let { shot, playerId, model, targetOf }: Props = $props()
  const arrowModel = $derived(model?.clone())

  /** How far past the target a miss carries before it is dropped. */
  const OVERSHOOT_METERS = 3

  let group: Group | undefined = $state()

  const from = new Vector3(shot.from.x, shot.from.y, shot.from.z)
  const aim = new Vector3(
    unwrapWorldXNear(from.x, shot.to.x),
    shot.to.y,
    shot.to.z
  )
  // A miss keeps the bearing it left with, carried past the target.
  const missTo = aim
    .clone()
    .sub(from)
    .normalize()
    .multiplyScalar(from.distanceTo(aim) + OVERSHOOT_METERS)
    .add(from)

  const target = new Vector3()
  const forward = new Vector3()
  // The model points down its own +X, as every weapon here does.
  const MODEL_AXIS = new Vector3(1, 0, 0)
  const facing = new Quaternion()

  useTask(() => {
    if (!group) return

    const elapsed = performance.now() - shot.launchedAt
    // A miss flies the same span plus its overshoot, so it stays in the air
    // proportionally longer rather than stopping short of where it is aimed.
    const span = shot.hit
      ? shot.flightMs
      : shot.flightMs *
        (from.distanceTo(missTo) / Math.max(from.distanceTo(aim), 0.001))
    const t = elapsed / Math.max(span, 1)

    if (t >= 1) {
      landArrow(playerId)
      return
    }

    // Killing shots keep their target alive visually until impact.
    const live = targetOf()
    if (!live) {
      landArrow(playerId)
      return
    }

    if (shot.hit) {
      target.set(unwrapWorldXNear(from.x, live.x), live.y, live.z)
    } else {
      target.copy(missTo)
    }

    group.position.lerpVectors(from, target, t)
    forward.subVectors(target, from)
    if (forward.lengthSq() > 1e-6) {
      facing.setFromUnitVectors(MODEL_AXIS, forward.normalize())
      group.quaternion.copy(facing)
    }
  })
</script>

{#if arrowModel}
  <T is={Group} bind:ref={group}>
    <T is={arrowModel} dispose={false} />
  </T>
{/if}

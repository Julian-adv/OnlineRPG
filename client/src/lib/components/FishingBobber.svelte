<script lang="ts">
  // Small red-and-white float; sine idle snaps to a faster jitter on a bite
  // so it reads from the isometric camera without sound. The bob mutates the
  // group ref directly — no per-frame reactive state.
  import { T, useTask } from '@threlte/core'
  import type { Group } from 'three'
  import type { BobberState } from '../stores/fishingStore'

  interface Props {
    bobber: BobberState
  }

  let { bobber }: Props = $props()

  let group: Group | undefined = $state()
  let t = 0
  useTask((delta) => {
    t += delta
    if (!group) return
    group.position.y = bobber.bite
      ? bobber.position.y - 0.25 + Math.sin(t * 18) * 0.08
      : bobber.position.y + 0.02 + Math.sin(t * 2.2) * 0.04
  })
</script>

<T.Group
  bind:ref={group}
  position={[bobber.position.x, bobber.position.y, bobber.position.z]}
>
  <T.Mesh position={[0, 0.06, 0]}>
    <T.SphereGeometry args={[0.09, 12, 12]} />
    <T.MeshStandardMaterial color="#d5493c" />
  </T.Mesh>
  <T.Mesh position={[0, -0.03, 0]}>
    <T.SphereGeometry args={[0.09, 12, 12]} />
    <T.MeshStandardMaterial color="#f2ede2" />
  </T.Mesh>
</T.Group>

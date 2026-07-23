<script lang="ts">
  // Placeholder shape on purpose (like the sword.png icons): a small red-and-
  // white float. The bob is a gentle sine idle that snaps into a deeper,
  // faster jitter on a bite — readable from the isometric camera without
  // any sound.
  import { T, useTask } from '@threlte/core'
  import type { BobberState } from '../stores/fishingStore'

  interface Props {
    bobber: BobberState
  }

  let { bobber }: Props = $props()

  let t = $state(0)
  useTask((delta) => {
    t += delta
  })

  const bobY = $derived(
    bobber.bite
      ? bobber.position.y - 0.25 + Math.sin(t * 18) * 0.08
      : bobber.position.y + 0.02 + Math.sin(t * 2.2) * 0.04
  )
</script>

<T.Group position={[bobber.position.x, bobY, bobber.position.z]}>
  <T.Mesh position={[0, 0.06, 0]}>
    <T.SphereGeometry args={[0.09, 12, 12]} />
    <T.MeshStandardMaterial color="#d5493c" />
  </T.Mesh>
  <T.Mesh position={[0, -0.03, 0]}>
    <T.SphereGeometry args={[0.09, 12, 12]} />
    <T.MeshStandardMaterial color="#f2ede2" />
  </T.Mesh>
</T.Group>

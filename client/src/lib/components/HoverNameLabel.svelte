<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import type { Position } from '../utils/movementUtils'
  import { billboardScale } from '../utils/billboardScale'
  import { makeTextBadge, NAME_BADGE_STYLE } from '../utils/textBadge'

  interface Props {
    text: string
    /** Anchor at the prop's origin; the label floats `labelY` above it. */
    position: Position
    labelY: number
    camera: THREE.Camera
  }

  let { text, position, labelY, camera }: Props = $props()

  const GAP = 0.06
  // Badge textures are cached and shared — never disposed here.
  const badge = $derived(makeTextBadge(text, NAME_BADGE_STYLE))
  let sprite = $state<THREE.Sprite | undefined>(undefined)
  const anchor = new THREE.Vector3()
  // Feedback only: raycasting it would make the label its own hover target.
  const NO_RAYCAST = () => {}

  /** Called from the render loop; also on mount so the first frame is sized. */
  export function update() {
    if (!sprite) return
    anchor.set(position.x, position.y + labelY, position.z)
    const s = billboardScale(camera.position.distanceTo(anchor))
    sprite.scale.set(badge.width * s, badge.height * s, 1)
    sprite.position.set(
      anchor.x,
      anchor.y + GAP + (badge.height * s) / 2,
      anchor.z
    )
  }

  $effect(() => {
    void badge
    void sprite
    update()
  })
</script>

<T.Sprite bind:ref={sprite} renderOrder={4} raycast={NO_RAYCAST}>
  <T.SpriteMaterial
    map={badge.texture}
    transparent={true}
    depthWrite={false}
    depthTest={false}
  />
</T.Sprite>

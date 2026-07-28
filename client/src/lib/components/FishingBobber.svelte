<script lang="ts">
  // Small red-and-white float; sine idle snaps to a faster jitter on a bite
  // so it reads from the isometric camera without sound. Stays hidden for
  // `landsInMs` (the cast swing + flight, matching the splash sound), then
  // splashes in at the cast point. During the fight it chases the
  // (server-simulated) fish — position is smoothed here each frame, never
  // snapped to the 4 Hz beats — and a splash of droplets marks its
  // thrashing, intensity following the fish's remaining stamina. A sagging
  // white line connects the angler's rod tip to the float. All motion
  // mutates refs directly — no per-frame reactive state.
  import { T, useTask } from '@threlte/core'
  import {
    BufferGeometry,
    Float32BufferAttribute,
    Group,
    Line,
    Points,
  } from 'three'
  import type { BobberState } from '../stores/fishingStore'

  interface Props {
    bobber: BobberState
    /** The owning angler's live position (mutated in place upstream);
     *  without it the line simply isn't drawn. */
    angler?: { x: number; y: number; z: number }
    /** World position of the angler's actual rod tip, queried per frame;
     *  null (model or rod not loaded yet) falls back to an approximation. */
    rodTipOf?: () => { x: number; y: number; z: number } | null
  }

  let { bobber, angler, rodTipOf }: Props = $props()

  const SPLASH_COUNT = 14
  const LINE_POINTS = 12
  const ROD_TIP_HEIGHT = 2.0
  const ROD_TIP_REACH = 0.7

  // The spawn position and flight time are deliberately fixed at cast
  // (initial value only); later store updates just move the lerp target.
  // svelte-ignore state_referenced_locally
  const initialPosition = bobber.position
  // svelte-ignore state_referenced_locally
  let landsIn = bobber.landsInMs

  let group: Group | undefined = $state()
  let splash: Points | undefined = $state()
  let line: Line | undefined = $state()

  const splashGeometry = new BufferGeometry()
  const splashPositions = new Float32BufferAttribute(
    new Float32Array(SPLASH_COUNT * 3),
    3
  )
  splashGeometry.setAttribute('position', splashPositions)
  const lineGeometry = new BufferGeometry()
  const linePositions = new Float32BufferAttribute(
    new Float32Array(LINE_POINTS * 3),
    3
  )
  lineGeometry.setAttribute('position', linePositions)

  let t = 0
  useTask((delta) => {
    t += delta
    if (!group) return

    if (landsIn > 0) {
      landsIn -= delta * 1000
      group.visible = landsIn <= 0
      if (line) line.visible = group.visible
      if (splash) splash.visible = false
      return
    }
    group.visible = true

    // Chase the (possibly moving) server position; 6/s ≈ settled well inside
    // the 250 ms beat interval.
    const k = Math.min(1, delta * 6)
    group.position.x += (bobber.position.x - group.position.x) * k
    group.position.z += (bobber.position.z - group.position.z) * k

    const fighting = bobber.fight !== undefined
    const running = bobber.fight?.fishState === 'running'
    const exhausted = bobber.fight?.fishState === 'exhausted'
    if (bobber.bite || (fighting && !exhausted)) {
      // Hard jitter: bite dip, or a live fish dragging the float under.
      group.position.y = bobber.position.y - 0.25 + Math.sin(t * 18) * 0.08
    } else {
      group.position.y = bobber.position.y + 0.02 + Math.sin(t * 2.2) * 0.04
    }

    if (splash) {
      // Splash droplets orbit the float while the fish thrashes; a rested or
      // exhausted fish barely ripples.
      const intensity = running ? 0.4 + (bobber.fight!.stamina / 100) * 0.6 : 0
      splash.visible = intensity > 0
      if (splash.visible) {
        const pos = splashPositions
        for (let i = 0; i < SPLASH_COUNT; i++) {
          const angle = (i / SPLASH_COUNT) * Math.PI * 2 + t * 3
          const radius = 0.25 + 0.2 * Math.sin(t * 7 + i * 2.1)
          pos.setXYZ(
            i,
            Math.cos(angle) * radius * intensity,
            Math.abs(Math.sin(t * 9 + i)) * 0.3 * intensity,
            Math.sin(angle) * radius * intensity
          )
        }
        pos.needsUpdate = true
      }
    }

    if (!line) return
    line.visible = angler !== undefined
    if (!angler) return
    // Anchor at the equipped rod's real tip; fall back to an approximation
    // above the angler leaning toward the float while the model loads. The
    // line sags toward the water on a simple quadratic.
    const dx = group.position.x - angler.x
    const dz = group.position.z - angler.z
    const horiz = Math.max(0.001, Math.sqrt(dx * dx + dz * dz))
    const tip = rodTipOf?.() ?? null
    const sx = tip ? tip.x : angler.x + (dx / horiz) * ROD_TIP_REACH
    const sy = tip ? tip.y : angler.y + ROD_TIP_HEIGHT
    const sz = tip ? tip.z : angler.z + (dz / horiz) * ROD_TIP_REACH
    const ex = group.position.x
    const ey = group.position.y + 0.12
    const ez = group.position.z
    const sag = Math.min(0.45, horiz * 0.06)
    const pos = linePositions
    for (let i = 0; i < LINE_POINTS; i++) {
      const p = i / (LINE_POINTS - 1)
      pos.setXYZ(
        i,
        sx + (ex - sx) * p,
        sy + (ey - sy) * p - sag * 4 * p * (1 - p),
        sz + (ez - sz) * p
      )
    }
    pos.needsUpdate = true
  })
</script>

<T.Group
  bind:ref={group}
  visible={false}
  position={[initialPosition.x, initialPosition.y, initialPosition.z]}
>
  <T.Mesh position={[0, 0.06, 0]}>
    <T.SphereGeometry args={[0.09, 12, 12]} />
    <T.MeshStandardMaterial color="#d5493c" />
  </T.Mesh>
  <T.Mesh position={[0, -0.03, 0]}>
    <T.SphereGeometry args={[0.09, 12, 12]} />
    <T.MeshStandardMaterial color="#f2ede2" />
  </T.Mesh>
  <T.Points bind:ref={splash} visible={false} geometry={splashGeometry}>
    <T.PointsMaterial
      color="#dff1fa"
      size={0.06}
      transparent
      opacity={0.85}
      depthWrite={false}
    />
  </T.Points>
</T.Group>

<T.Line
  bind:ref={line}
  visible={false}
  geometry={lineGeometry}
  frustumCulled={false}
>
  <T.LineBasicMaterial color="#f4f8fb" transparent opacity={0.65} />
</T.Line>

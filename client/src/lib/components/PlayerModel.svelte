<script lang="ts">
  import { T } from '@threlte/core'
  import { Text } from '@threlte/extras'
  import type { Vector3, Mesh } from 'three'

  interface Props {
    position: Vector3 | [number, number, number]
    name: string
    isCurrentPlayer: boolean
    onmove?: (detail: { x: number; y: number; z: number }) => void
  }

  let { position, name, isCurrentPlayer, onmove }: Props = $props()

  const positionArray = $derived(
    Array.isArray(position)
      ? position
      : ([position.x, position.y, position.z] as [number, number, number])
  )

  let meshRef = $state<Mesh | undefined>(undefined)
  let isDragging = $state(false)
  let dragStart = $state({ x: 0, y: 0 })

  function handlePointerDown(event: PointerEvent) {
    if (!isCurrentPlayer) return
    isDragging = true
    dragStart = { x: event.clientX, y: event.clientY }
  }

  function handlePointerMove(event: PointerEvent) {
    if (!isDragging || !isCurrentPlayer) return

    const deltaX = (event.clientX - dragStart.x) * 0.01
    const deltaZ = (event.clientY - dragStart.y) * 0.01

    const pos = Array.isArray(position)
      ? { x: position[0], y: position[1], z: position[2] }
      : { x: position.x, y: position.y, z: position.z }

    const newPosition = {
      x: pos.x + deltaX,
      y: pos.y || 1,
      z: pos.z + deltaZ,
    }

    onmove?.(newPosition)
    dragStart = { x: event.clientX, y: event.clientY }
  }

  function handlePointerUp() {
    isDragging = false
  }

  function handleClick() {
    if (!isCurrentPlayer) return
    console.log(`Clicked on ${name}`)
  }
</script>

<svelte:window
  on:pointermove={handlePointerMove}
  on:pointerup={handlePointerUp}
/>

<T.Group position={positionArray}>
  <!-- Player body (capsule-like shape) -->
  <T.Mesh
    bind:ref={meshRef}
    castShadow
    receiveShadow
    on:pointerdown={handlePointerDown}
    on:click={handleClick}
  >
    <T.CapsuleGeometry args={[0.3, 1.2, 8, 16]} />
    <T.MeshLambertMaterial color={isCurrentPlayer ? '#4299e1' : '#ed8936'} />
  </T.Mesh>

  <!-- Player head -->
  <T.Mesh position={[0, 1, 0]} castShadow>
    <T.SphereGeometry args={[0.25, 16, 16]} />
    <T.MeshLambertMaterial color={isCurrentPlayer ? '#2b6cb0' : '#c05621'} />
  </T.Mesh>

  <!-- Name tag -->
  <Text
    text={name}
    position={[0, 2.2, 0]}
    fontSize={0.3}
    color={isCurrentPlayer ? '#4299e1' : '#ffffff'}
    anchorX="center"
    anchorY="middle"
  />

  <!-- Selection indicator for current player -->
  {#if isCurrentPlayer}
    <T.Mesh position={[0, -0.1, 0]} rotation={[-Math.PI / 2, 0, 0]}>
      <T.RingGeometry args={[0.8, 1.0, 16]} />
      <T.MeshBasicMaterial color="#4299e1" transparent opacity={0.5} />
    </T.Mesh>
  {/if}
</T.Group>

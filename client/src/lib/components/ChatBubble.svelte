<script lang="ts">
  import { T } from '@threlte/core'
  import TextLabel from './TextLabel.svelte'
  import { truncateGraphemes } from '../utils/textWrap'
  import { billboardScale, billboardZoomT } from '../utils/billboardScale'
  import type { Vector3 } from 'three'
  import * as THREE from 'three'

  interface Props {
    position: Vector3
    camera: THREE.Camera | undefined
    message: string
  }

  let { position, camera, message }: Props = $props()

  const _scratchVec = new THREE.Vector3()
  const OVERLAY_RENDER_ORDER = 9999

  const PADDING_X = 0.6
  const PADDING_Y = 0.3
  const MAX_TEXT_WIDTH = 5
  const MAX_BUBBLE_WIDTH = MAX_TEXT_WIDTH + PADDING_X
  const MAX_DISPLAY_CHARS = 300

  let textBounds = $state({ width: 1, height: 0.3 })

  let textRef = $state<TextLabel | null>(null)
  let bubbleGroup = $state<THREE.Group | undefined>(undefined)

  function handleTextSync() {
    if (textRef?.textRenderInfo?.blockBounds) {
      const [minX, minY, maxX, maxY] = textRef.textRenderInfo.blockBounds
      textBounds = {
        width: maxX - minX,
        height: maxY - minY,
      }
    }
  }

  export function update() {
    if (!bubbleGroup || !camera) return

    // Use a fixed approximate height to avoid circular dependency
    _scratchVec.set(position.x, position.y + 2.0, position.z)
    const dist = camera.position.distanceTo(_scratchVec)

    const minHeight = 1.9
    const maxHeight = 2.6

    const currentScale = billboardScale(dist)
    const heightOffset =
      minHeight + billboardZoomT(dist) * (maxHeight - minHeight)

    bubbleGroup.scale.set(currentScale, currentScale, currentScale)
    bubbleGroup.position.set(position.x, position.y + heightOffset, position.z)

    // Update Rotation
    // Make the bubble parallel to the camera screen plane
    // This handles X, Y, and Z rotations automatically and prevents distortion at screen edges
    bubbleGroup.quaternion.copy(camera.quaternion)
  }

  // Create rounded rectangle shape for chat bubble with tail
  function createRoundedRectShape(
    width: number,
    height: number,
    radius: number
  ): THREE.Shape {
    const shape = new THREE.Shape()
    const x = -width / 2
    const y = radius

    shape.moveTo(x + radius, y)
    // Bottom edge with curved tail in the center
    shape.lineTo(-radius, y)
    shape.quadraticCurveTo(0, y, 0, y - radius)
    shape.quadraticCurveTo(0, y, radius, y)
    shape.lineTo(x + width - radius, y)
    // Right edge
    shape.quadraticCurveTo(x + width, y, x + width, y + radius)
    shape.lineTo(x + width, y + height - radius)
    // Top edge
    shape.quadraticCurveTo(
      x + width,
      y + height,
      x + width - radius,
      y + height
    )
    shape.lineTo(x + radius, y + height)
    // Left edge
    shape.quadraticCurveTo(x, y + height, x, y + height - radius)
    shape.lineTo(x, y + radius)
    shape.quadraticCurveTo(x, y, x + radius, y)

    return shape
  }

  // Create line geometry from shape for border (closed loop)
  function createBorderGeometry(shape: THREE.Shape): THREE.BufferGeometry {
    const points = shape.getPoints(32)
    points.push(points[0]) // close the loop
    const geometry = new THREE.BufferGeometry().setFromPoints(points)
    return geometry
  }

  const bubbleWidth = $derived(
    Math.min(textBounds.width + PADDING_X, MAX_BUBBLE_WIDTH)
  )
  const bubbleHeight = $derived(textBounds.height + PADDING_Y)
  const cornerRadius = 0.1
  const bubbleShape = $derived(
    createRoundedRectShape(bubbleWidth, bubbleHeight, cornerRadius)
  )
  const bubbleBorderGeometry = $derived(createBorderGeometry(bubbleShape))
  const bubbleCenterY = $derived(cornerRadius + bubbleHeight / 2)
  const displayText = $derived.by(() => {
    const truncated = truncateGraphemes(message, MAX_DISPLAY_CHARS)
    return truncated.length < message.length ? truncated + '...' : message
  })
</script>

<!-- Chat bubble background -->
<T.Group bind:ref={bubbleGroup}>
  <T.Mesh position={[0, cornerRadius, 0]} renderOrder={OVERLAY_RENDER_ORDER}>
    <T.ShapeGeometry args={[bubbleShape]} />
    <T.MeshBasicMaterial
      color="#000000"
      opacity={0.85}
      transparent={true}
      depthTest={false}
    />
  </T.Mesh>

  <!-- Chat bubble border (white line) -->
  <T.Line
    position={[0, cornerRadius, 0.001]}
    renderOrder={OVERLAY_RENDER_ORDER}
  >
    <T is={bubbleBorderGeometry} />
    <T.LineBasicMaterial color="#ffffff" depthTest={false} />
  </T.Line>

  <!-- Chat bubble text -->
  <TextLabel
    bind:this={textRef}
    text={displayText}
    position={[0, bubbleCenterY + cornerRadius, 0.01]}
    fontSize={0.25}
    color="#ffffff"
    outlineColor="#000000"
    outlineWidth={7}
    anchorX="center"
    anchorY="middle"
    maxWidth={MAX_TEXT_WIDTH}
    onsync={handleTextSync}
    depthTest={false}
    renderOrder={OVERLAY_RENDER_ORDER}
  />
</T.Group>

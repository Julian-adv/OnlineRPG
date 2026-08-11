<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { MeshBasicNodeMaterial } from 'three/webgpu'
  import { onDestroy } from 'svelte'
  import { wrapLines } from '../utils/textWrap'

  interface Props {
    text: string
    fontSize?: number
    color?: string
    outlineColor?: string
    outlineWidth?: number
    anchorX?: 'left' | 'center' | 'right'
    anchorY?: 'top' | 'middle' | 'bottom'
    fillOpacity?: number
    maxWidth?: number
    overflowWrap?: 'normal' | 'break-word'
    whiteSpace?: 'normal' | 'nowrap'
    depthOffset?: number
    depthTest?: boolean
    renderOrder?: number
    onsync?: () => void
    position?: [number, number, number]
    'position.y'?: number
  }

  let {
    text,
    fontSize = 0.3,
    color = '#ffffff',
    outlineColor,
    outlineWidth = 0,
    anchorX = 'center',
    anchorY = 'middle',
    fillOpacity = 1.0,
    maxWidth,
    overflowWrap = 'normal',
    whiteSpace = 'normal',
    depthOffset,
    depthTest = true,
    renderOrder,
    onsync,
    position = [0, 0, 0],
    'position.y': positionY,
  }: Props = $props()

  const PIXELS_PER_UNIT = 256

  // Exported for ChatBubble compatibility (bind:this → ref.textRenderInfo.blockBounds)
  export const textRenderInfo = { blockBounds: [0, 0, 0, 0] as number[] }

  let canvas = document.createElement('canvas')
  let ctx = canvas.getContext('2d')!

  let texture = new THREE.CanvasTexture(canvas)
  texture.minFilter = THREE.LinearFilter
  texture.magFilter = THREE.LinearFilter

  // Track previous canvas dimensions. When they change we must create a
  // new canvas + CanvasTexture so the WebGPU backend allocates a GPUTexture
  // with the correct size (it only creates the GPUTexture once per Texture).
  // A new canvas is needed because the old texture still references the old
  // canvas — resizing a shared canvas would corrupt the old GPUTexture.
  let prevCanvasW = 0
  let prevCanvasH = 0

  let worldWidth = $state(0.01)
  let worldHeight = $state(0.01)
  let anchorOffsetX = $state(0)
  let anchorOffsetY = $state(0)

  const material = new MeshBasicNodeMaterial()
  material.map = texture
  material.transparent = true
  material.depthWrite = false
  $effect(() => {
    material.depthTest = depthTest
  })
  material.side = THREE.DoubleSide

  function wrapText(
    inputText: string,
    maxWidthPx: number | undefined,
    breakWord: boolean
  ): string[] {
    if (!maxWidthPx || whiteSpace === 'nowrap') {
      const paragraphs = inputText.split('\n')
      return paragraphs.length ? paragraphs : ['']
    }
    return wrapLines(
      inputText,
      maxWidthPx,
      breakWord,
      (s) => ctx.measureText(s).width
    )
  }

  function renderCanvas() {
    const pxFont = fontSize * PIXELS_PER_UNIT
    const font = `${pxFont}px sans-serif`
    ctx.font = font

    const maxWPx = maxWidth ? maxWidth * PIXELS_PER_UNIT : undefined
    const lines = wrapText(text, maxWPx, overflowWrap === 'break-word')
    const lineHeight = pxFont * 1.2

    let maxLineWidth = 0
    for (const line of lines) {
      maxLineWidth = Math.max(maxLineWidth, ctx.measureText(line).width)
    }

    const totalTextHeight = lines.length * lineHeight
    const outlinePad = Math.max(0, outlineWidth)
    const pad = 4 + outlinePad
    const cw = Math.max(1, Math.ceil(maxLineWidth + pad * 2))
    const ch = Math.max(1, Math.ceil(totalTextHeight + pad * 2))

    // WebGPU allocates a GPUTexture once per THREE.Texture and never resizes
    // it. If canvas dimensions change we must create a fresh canvas + texture
    // so the backend allocates a new GPUTexture with the correct size.
    // A new canvas is needed because the old texture still references the old one.
    if (cw !== prevCanvasW || ch !== prevCanvasH) {
      prevCanvasW = cw
      prevCanvasH = ch

      canvas = document.createElement('canvas')
      canvas.width = cw
      canvas.height = ch
      ctx = canvas.getContext('2d')!

      // Do NOT dispose the old texture (see onDestroy comment).
      texture = new THREE.CanvasTexture(canvas)
      texture.minFilter = THREE.LinearFilter
      texture.magFilter = THREE.LinearFilter
      material.map = texture
    }

    ctx.clearRect(0, 0, cw, ch)
    ctx.font = font
    ctx.lineJoin = 'round'
    ctx.lineCap = 'round'
    ctx.textBaseline = 'top'

    for (let i = 0; i < lines.length; i++) {
      let x = pad
      if (anchorX === 'center') {
        x = (cw - ctx.measureText(lines[i]).width) / 2
      } else if (anchorX === 'right') {
        x = cw - ctx.measureText(lines[i]).width - pad
      }
      if (outlineColor && outlineWidth > 0) {
        ctx.strokeStyle = outlineColor
        ctx.lineWidth = outlineWidth
        ctx.strokeText(lines[i], x, pad + i * lineHeight)
      }
      ctx.fillStyle = color
      ctx.fillText(lines[i], x, pad + i * lineHeight)
    }

    texture.needsUpdate = true

    worldWidth = cw / PIXELS_PER_UNIT
    worldHeight = ch / PIXELS_PER_UNIT

    if (anchorX === 'left') anchorOffsetX = worldWidth / 2
    else if (anchorX === 'right') anchorOffsetX = -worldWidth / 2
    else anchorOffsetX = 0

    if (anchorY === 'top') anchorOffsetY = -worldHeight / 2
    else if (anchorY === 'bottom') anchorOffsetY = worldHeight / 2
    else anchorOffsetY = 0

    // Troika-compatible blockBounds: [minX, minY, maxX, maxY] relative to anchor
    textRenderInfo.blockBounds = [
      -worldWidth / 2 + anchorOffsetX,
      -worldHeight / 2 + anchorOffsetY,
      worldWidth / 2 + anchorOffsetX,
      worldHeight / 2 + anchorOffsetY,
    ]

    onsync?.()
  }

  // Re-render canvas when visual properties change
  $effect(() => {
    renderCanvas()
  })

  // Update material opacity without re-rendering canvas
  $effect(() => {
    material.opacity = fillOpacity
  })

  // Update depth offset
  $effect(() => {
    if (depthOffset !== undefined) {
      material.polygonOffset = true
      material.polygonOffsetFactor = depthOffset
      material.polygonOffsetUnits = depthOffset
    }
  })

  let meshRef = $state<THREE.Mesh | undefined>(undefined)

  onDestroy(() => {
    // Hide mesh immediately so the renderer won't try to draw it.
    if (meshRef) meshRef.visible = false
    // Do NOT call texture.dispose() or material.dispose() here.
    // All MeshBasicNodeMaterial+CanvasTexture instances share a NodeBuilderState
    // whose original Sampler bindings hold a dispose listener on the texture.
    // Disposing the texture sets the original Sampler._texture = null, and
    // Binding.clone() copies _texture via Object.assign (bypassing the setter),
    // so all future cloned bindings inherit null — crashing createBindGroup
    // with "Invalid value used as weak map key".
    // Let GC reclaim the texture and material instead.
  })
</script>

<T.Mesh
  bind:ref={meshRef}
  {renderOrder}
  position={[
    (position[0] ?? 0) + anchorOffsetX,
    (positionY ?? position[1] ?? 0) + anchorOffsetY,
    position[2] ?? 0,
  ]}
>
  <T.PlaneGeometry args={[worldWidth, worldHeight]} />
  <T is={material} />
</T.Mesh>

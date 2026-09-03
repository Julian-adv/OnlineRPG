<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { MeshBasicNodeMaterial } from 'three/webgpu'
  import { onDestroy } from 'svelte'
  import { wrapLines } from '../utils/textWrap'
  import {
    fitLabelCanvas,
    labelPlane,
    measureCtx,
    releaseLabelCanvas,
    type LabelCanvas,
  } from '../utils/text-label-pool'

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
    whiteSpace?: 'normal' | 'nowrap'
    depthOffset?: number
    depthTest?: boolean
    renderOrder?: number
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
    whiteSpace = 'normal',
    depthOffset,
    depthTest = true,
    renderOrder,
    position = [0, 0, 0],
    'position.y': positionY,
  }: Props = $props()

  const PIXELS_PER_UNIT = 256

  let label: LabelCanvas | null = null

  let worldWidth = $state(0.01)
  let worldHeight = $state(0.01)
  let anchorOffsetX = $state(0)
  let anchorOffsetY = $state(0)

  const material = new MeshBasicNodeMaterial()
  material.transparent = true
  material.depthWrite = false
  $effect(() => {
    material.depthTest = depthTest
  })
  material.side = THREE.DoubleSide

  function renderCanvas() {
    const pxFont = fontSize * PIXELS_PER_UNIT
    const font = `${pxFont}px sans-serif`
    measureCtx.font = font

    const lines =
      maxWidth && whiteSpace !== 'nowrap'
        ? wrapLines(
            text,
            maxWidth * PIXELS_PER_UNIT,
            (s) => measureCtx.measureText(s).width
          )
        : text.split('\n')
    const lineHeight = pxFont * 1.2

    const widths = lines.map((line) => measureCtx.measureText(line).width)
    const maxLineWidth = Math.max(...widths)

    const totalTextHeight = lines.length * lineHeight
    const outlinePad = Math.max(0, outlineWidth)
    const pad = 4 + outlinePad
    const cw = Math.max(1, Math.ceil(maxLineWidth + pad * 2))
    const ch = Math.max(1, Math.ceil(totalTextHeight + pad * 2))
    label = fitLabelCanvas(label, cw, ch)
    const { ctx, texture } = label
    material.map = texture

    ctx.clearRect(0, 0, cw + 1, ch + 1)
    ctx.font = font
    ctx.lineJoin = 'round'
    ctx.lineCap = 'round'
    ctx.textBaseline = 'top'

    for (let i = 0; i < lines.length; i++) {
      let x = pad
      if (anchorX === 'center') {
        x = (cw - widths[i]) / 2
      } else if (anchorX === 'right') {
        x = cw - widths[i] - pad
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
  }

  $effect(() => {
    renderCanvas()
  })

  $effect(() => {
    material.opacity = fillOpacity
  })

  $effect(() => {
    if (depthOffset !== undefined) {
      material.polygonOffset = true
      material.polygonOffsetFactor = depthOffset
      material.polygonOffsetUnits = depthOffset
    }
  })

  let meshRef = $state<THREE.Mesh | undefined>(undefined)

  onDestroy(() => {
    if (meshRef) meshRef.visible = false
    // Never dispose the texture: the shared NodeBuilderState sampler binding
    // would null its _texture and crash later createBindGroup calls.
    if (label) releaseLabelCanvas(label)
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
  scale={[worldWidth, worldHeight, 1]}
>
  <T is={labelPlane} dispose={false} />
  <T is={material} />
</T.Mesh>

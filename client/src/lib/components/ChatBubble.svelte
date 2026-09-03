<script lang="ts">
  import { useTask, useThrelte } from '@threlte/core'
  import * as THREE from 'three'
  import { truncateGraphemes } from '../utils/textWrap'
  import { billboardScale, billboardZoomT } from '../utils/billboardScale'
  import { bubbleLayer } from '../stores/bubbleLayerStore'

  interface Props {
    position: THREE.Vector3
    camera: THREE.OrthographicCamera | undefined
    message: string
  }

  let { position, camera, message }: Props = $props()

  const { size, renderStage } = useThrelte()

  // DOM rather than a mesh so it can paint over the minimap. CSS is authored
  // at this many px per world unit; --s scales sizes (not a transform, so the
  // 1px outline stays crisp) to the projected size.
  const PX_PER_UNIT = 64
  const MAX_DISPLAY_CHARS = 300
  const RADIUS = 6
  const TAIL = 7
  const MIN_HEIGHT = 1.9
  const MAX_HEIGHT = 2.6

  const _anchor = new THREE.Vector3()
  let el = $state<HTMLDivElement>()
  let boxEl = $state<HTMLDivElement>()
  let scale = $state(1)
  let boxW = $state(0)
  let boxH = $state(0)
  let lastX = NaN
  let lastY = NaN

  const tailH = $derived(Math.round(TAIL * scale))

  const displayText = $derived.by(() => {
    const truncated = truncateGraphemes(message, MAX_DISPLAY_CHARS)
    return truncated.length < message.length ? truncated + '...' : message
  })

  // Outline and tail are one path so both edges rasterize identically.
  const shapePath = $derived.by(() => {
    const r = RADIUS * scale
    const half = TAIL * scale
    const cx = boxW / 2
    const x1 = boxW - 0.5
    const y1 = boxH - 0.5
    return (
      `M${0.5 + r} 0.5H${x1 - r}Q${x1} 0.5 ${x1} ${0.5 + r}V${y1 - r}` +
      `Q${x1} ${y1} ${x1 - r} ${y1}H${cx + half}` +
      `Q${cx} ${y1} ${cx} ${y1 + tailH}Q${cx} ${y1} ${cx - half} ${y1}` +
      `H${0.5 + r}Q0.5 ${y1} 0.5 ${y1 - r}V${0.5 + r}Q0.5 0.5 ${0.5 + r} 0.5Z`
    )
  })

  // Reparented into the HUD layer, so .bubble must stay this component's
  // only root node: Svelte tears down by walking siblings between its roots.
  $effect(() => {
    const layer = $bubbleLayer
    const node = el
    if (!layer || !node) return
    layer.append(node)
    return () => node.remove()
  })

  $effect(() => {
    if (!boxEl) return
    const observer = new ResizeObserver(([entry]) => {
      const box = entry.borderBoxSize[0]
      boxW = box.inlineSize
      boxH = box.blockSize
    })
    observer.observe(boxEl)
    return () => observer.disconnect()
  })

  useTask(
    () => {
      if (!el || !camera) return
      _anchor.set(position.x, position.y + 2.0, position.z)
      const dist = camera.position.distanceTo(_anchor)
      _anchor.y =
        position.y +
        MIN_HEIGHT +
        billboardZoomT(dist) * (MAX_HEIGHT - MIN_HEIGHT)
      camera.updateMatrixWorld()
      _anchor.project(camera)
      const { width, height } = size.current
      const x = Math.round(((_anchor.x + 1) * width) / 2)
      const y = Math.round(((1 - _anchor.y) * height) / 2) - tailH
      if (x !== lastX || y !== lastY) {
        el.style.translate = `${x}px ${y}px`
        lastX = x
        lastY = y
      }
      const pxPerUnit = (camera.projectionMatrix.elements[5] * height) / 2
      // Quantized so text reflows only on real zoom steps, not every frame.
      scale =
        Math.round(((billboardScale(dist) * pxPerUnit) / PX_PER_UNIT) * 32) / 32
    },
    { stage: renderStage, autoInvalidate: false }
  )
</script>

<div class="bubble" bind:this={el} style:--s={scale}>
  <svg class="shape"><path d={shapePath} /></svg>
  <div class="box" bind:this={boxEl}>{displayText}</div>
</div>

<style>
  .bubble {
    --s: 1;
    position: absolute;
    left: 0;
    top: 0;
    width: max-content;
    transform: translate(-50%, -100%);
  }

  .shape {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
    fill: rgba(0, 0, 0, 0.55);
    stroke: #fff;
    stroke-width: 1;
  }

  .box {
    position: relative;
    max-width: calc(320px * var(--s));
    padding: calc(round(8px * var(--s), 1px) + 1px)
      calc(round(16px * var(--s), 1px) + 1px);
    color: #fff;
    font-family: sans-serif;
    font-size: calc(16px * var(--s));
    line-height: round(19.2px * var(--s), 1px);
    text-align: center;
    text-shadow: 0 0 calc(2px * var(--s)) #000;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>

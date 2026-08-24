import type { MapLabelDef, MapLabelKind } from '../data/mapLabels'

export interface ScreenPoint {
  x: number
  y: number
}

export interface ScreenSize {
  width: number
  height: number
}

export interface ScreenRect {
  left: number
  top: number
  right: number
  bottom: number
}

export interface MapLabelLayoutInput<T extends MapLabelDef = MapLabelDef> {
  label: T
  anchor: ScreenPoint
  textSize?: ScreenSize
  markerRadius?: number
}

export interface PositionedMapLabel<T extends MapLabelDef = MapLabelDef> {
  label: T
  anchor: ScreenPoint
  textCenter: ScreenPoint
  textOffset: ScreenPoint
  textBounds: ScreenRect | null
  textVisible: boolean
}

export interface MapLabelLayoutOptions {
  zoomSpan: number
  areaZoomSpan?: number
  viewport: ScreenRect
  reservedBounds?: readonly ScreenRect[]
  collisionPadding?: number
  edgePadding?: number
  markerGap?: number
}

interface OccupiedRect {
  bounds: ScreenRect
  ownerIndex?: number
  priority: number
}

export const MAP_LABEL_ZOOM_RANGE = {
  continent: { min: 8, max: Infinity },
  sea: { min: 4, max: Infinity },
  capital: { min: 1, max: Infinity },
  city: { min: 1, max: 24 },
  town: { min: 1, max: 16 },
  island: { min: 1, max: 16 },
  dungeon: { min: 1, max: 24 },
} as const satisfies Record<MapLabelKind, { min: number; max: number }>

export const MAP_LABEL_TEXT_ZOOM_RANGE = {
  continent: { min: 8, max: Infinity },
  sea: { min: 4, max: Infinity },
  capital: { min: 1, max: Infinity },
  city: { min: 1, max: 16 },
  town: { min: 1, max: 8 },
  island: { min: 1, max: 16 },
  dungeon: { min: 1, max: 3 },
} as const satisfies Record<MapLabelKind, { min: number; max: number }>

export const MAP_LABEL_PRIORITY = {
  continent: 800,
  capital: 700,
  city: 600,
  town: 500,
  island: 400,
  sea: 300,
  dungeon: 100,
} as const satisfies Record<MapLabelKind, number>

const FIXED_KINDS = new Set<MapLabelKind>(['continent', 'sea', 'island'])

const DEFAULT_MARKER_RADIUS = {
  continent: 0,
  sea: 0,
  island: 0,
  capital: 7,
  city: 6,
  town: 6,
  dungeon: 6,
} as const satisfies Record<MapLabelKind, number>

const TEXT_STYLE = {
  continent: { fontSize: 22, widthFactor: 0.68, letterSpacing: 4 },
  sea: { fontSize: 15, widthFactor: 0.58, letterSpacing: 2 },
  island: { fontSize: 13, widthFactor: 0.6, letterSpacing: 0.4 },
  capital: { fontSize: 16, widthFactor: 0.62, letterSpacing: 0.3 },
  city: { fontSize: 14, widthFactor: 0.62, letterSpacing: 0.2 },
  town: { fontSize: 13, widthFactor: 0.62, letterSpacing: 0.2 },
  dungeon: { fontSize: 13, widthFactor: 0.6, letterSpacing: 0.2 },
} as const satisfies Record<
  MapLabelKind,
  { fontSize: number; widthFactor: number; letterSpacing: number }
>

export function isMapLabelVisibleAtZoom(
  kind: MapLabelKind,
  zoomSpan: number
): boolean {
  const range = MAP_LABEL_ZOOM_RANGE[kind]
  return zoomSpan >= range.min && zoomSpan <= range.max
}

export function isMapLabelTextVisibleAtZoom(
  kind: MapLabelKind,
  zoomSpan: number
): boolean {
  const range = MAP_LABEL_TEXT_ZOOM_RANGE[kind]
  return zoomSpan >= range.min && zoomSpan <= range.max
}

export function isFixedMapLabel(kind: MapLabelKind): boolean {
  return FIXED_KINDS.has(kind)
}

export function estimateMapLabelTextSize(
  name: string,
  kind: MapLabelKind
): ScreenSize {
  const style = TEXT_STYLE[kind]
  const glyphWidth = name.length * style.fontSize * style.widthFactor
  const spacing = Math.max(0, name.length - 1) * style.letterSpacing
  return {
    width: Math.ceil(glyphWidth + spacing),
    height: Math.ceil(style.fontSize * 1.3),
  }
}

export function getMapLabelCandidateOffsets(
  kind: MapLabelKind,
  textSize: ScreenSize,
  markerGap = 12
): readonly ScreenPoint[] {
  if (isFixedMapLabel(kind)) return [{ x: 0, y: 0 }]

  const horizontal = markerGap + textSize.width / 2
  const vertical = markerGap + textSize.height / 2
  return [
    { x: horizontal, y: 0 },
    { x: 0, y: -vertical },
    { x: 0, y: vertical },
    { x: -horizontal, y: 0 },
    { x: horizontal, y: -vertical },
    { x: -horizontal, y: -vertical },
    { x: horizontal, y: vertical },
    { x: -horizontal, y: vertical },
  ]
}

export function layoutMapLabels<T extends MapLabelDef>(
  inputs: readonly MapLabelLayoutInput<T>[],
  options: MapLabelLayoutOptions
): PositionedMapLabel<T>[] {
  const collisionPadding = options.collisionPadding ?? 2
  const edgePadding = options.edgePadding ?? 6
  const markerGap = options.markerGap ?? 12
  const visible = inputs
    .map((input, sourceIndex) => ({ input, sourceIndex }))
    .filter(({ input }) =>
      isMapLabelVisibleAtZoom(
        input.label.kind,
        getLabelZoomSpan(input.label.kind, options)
      )
    )
  const occupied: OccupiedRect[] = (options.reservedBounds ?? []).map(
    (bounds) => ({ bounds, priority: Infinity })
  )
  const placed = new Map<number, PositionedMapLabel<T>>()

  for (const { input, sourceIndex } of visible) {
    const radius = input.markerRadius ?? DEFAULT_MARKER_RADIUS[input.label.kind]
    if (radius <= 0) continue
    occupied.push({
      bounds: {
        left: input.anchor.x - radius,
        top: input.anchor.y - radius,
        right: input.anchor.x + radius,
        bottom: input.anchor.y + radius,
      },
      ownerIndex: sourceIndex,
      priority: MAP_LABEL_PRIORITY[input.label.kind],
    })
  }

  const ordered = [...visible].sort((a, b) => {
    const priorityDelta =
      MAP_LABEL_PRIORITY[b.input.label.kind] -
      MAP_LABEL_PRIORITY[a.input.label.kind]
    if (priorityDelta !== 0) return priorityDelta
    return a.sourceIndex - b.sourceIndex
  })

  for (const { input, sourceIndex } of ordered) {
    if (
      !isMapLabelTextVisibleAtZoom(
        input.label.kind,
        getLabelZoomSpan(input.label.kind, options)
      )
    ) {
      placed.set(sourceIndex, hiddenResult(input))
      continue
    }
    const size =
      input.textSize ??
      estimateMapLabelTextSize(input.label.name, input.label.kind)
    const offsets = getMapLabelCandidateOffsets(
      input.label.kind,
      size,
      markerGap
    )
    const priority = MAP_LABEL_PRIORITY[input.label.kind]
    let result = hiddenResult(input)

    for (const relaxLowerPriority of [false, true]) {
      for (const offset of offsets) {
        const center = {
          x: input.anchor.x + offset.x,
          y: input.anchor.y + offset.y,
        }
        const bounds = rectFromCenter(center, size)
        if (!isInside(bounds, options.viewport, edgePadding)) continue
        const blocked = occupied.some((other) => {
          if (other.ownerIndex === sourceIndex) return false
          if (relaxLowerPriority && other.priority < priority) return false
          return rectsOverlap(bounds, other.bounds, collisionPadding)
        })
        if (blocked) continue
        occupied.push({ bounds, priority })
        result = {
          label: input.label,
          anchor: { ...input.anchor },
          textCenter: center,
          textOffset: offset,
          textBounds: bounds,
          textVisible: true,
        }
        break
      }
      if (result.textVisible) break
    }
    placed.set(sourceIndex, result)
  }

  return visible.map(({ sourceIndex }) => placed.get(sourceIndex)!)
}

function getLabelZoomSpan(
  kind: MapLabelKind,
  options: MapLabelLayoutOptions
): number {
  return isFixedMapLabel(kind)
    ? (options.areaZoomSpan ?? options.zoomSpan)
    : options.zoomSpan
}

function hiddenResult<T extends MapLabelDef>(
  input: MapLabelLayoutInput<T>
): PositionedMapLabel<T> {
  return {
    label: input.label,
    anchor: { ...input.anchor },
    textCenter: { ...input.anchor },
    textOffset: { x: 0, y: 0 },
    textBounds: null,
    textVisible: false,
  }
}

function rectFromCenter(center: ScreenPoint, size: ScreenSize): ScreenRect {
  return {
    left: center.x - size.width / 2,
    top: center.y - size.height / 2,
    right: center.x + size.width / 2,
    bottom: center.y + size.height / 2,
  }
}

function isInside(rect: ScreenRect, viewport: ScreenRect, padding: number) {
  return (
    rect.left >= viewport.left + padding &&
    rect.top >= viewport.top + padding &&
    rect.right <= viewport.right - padding &&
    rect.bottom <= viewport.bottom - padding
  )
}

function rectsOverlap(a: ScreenRect, b: ScreenRect, padding: number) {
  return (
    a.left - padding < b.right + padding &&
    a.right + padding > b.left - padding &&
    a.top - padding < b.bottom + padding &&
    a.bottom + padding > b.top - padding
  )
}

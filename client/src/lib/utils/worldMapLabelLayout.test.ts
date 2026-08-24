import { describe, expect, it } from 'vitest'
import type { MapLabelDef, MapLabelKind } from '../data/mapLabels'
import {
  getMapLabelCandidateOffsets,
  isMapLabelTextVisibleAtZoom,
  isMapLabelVisibleAtZoom,
  layoutMapLabels,
} from './worldMapLabelLayout'

const viewport = { left: 0, top: 0, right: 400, bottom: 240 }

function label(id: string, kind: MapLabelKind, x = 0, z = 0): MapLabelDef {
  return { id, name: id, kind, x, z }
}

describe('world map label zoom hierarchy', () => {
  it('keeps distant point markers while reducing lower-tier text', () => {
    expect(isMapLabelVisibleAtZoom('city', 1)).toBe(true)
    expect(isMapLabelVisibleAtZoom('city', 24)).toBe(true)
    expect(isMapLabelVisibleAtZoom('city', 25)).toBe(false)
    expect(isMapLabelVisibleAtZoom('island', 16)).toBe(true)
    expect(isMapLabelVisibleAtZoom('island', 17)).toBe(false)
    expect(isMapLabelVisibleAtZoom('sea', 3)).toBe(false)
    expect(isMapLabelVisibleAtZoom('sea', 4)).toBe(true)
    expect(isMapLabelVisibleAtZoom('continent', 7)).toBe(false)
    expect(isMapLabelVisibleAtZoom('continent', 8)).toBe(true)
    expect(isMapLabelTextVisibleAtZoom('town', 8)).toBe(true)
    expect(isMapLabelTextVisibleAtZoom('town', 9)).toBe(false)
    expect(isMapLabelTextVisibleAtZoom('dungeon', 3)).toBe(true)
    expect(isMapLabelTextVisibleAtZoom('dungeon', 4)).toBe(false)
  })
})

describe('world map label candidates', () => {
  it('keeps area and sea labels centered on their authored coordinates', () => {
    const continent = getMapLabelCandidateOffsets(
      'continent',
      { width: 100, height: 24 },
      12
    )
    const sea = getMapLabelCandidateOffsets(
      'sea',
      { width: 80, height: 20 },
      12
    )

    expect(continent).toEqual([{ x: 0, y: 0 }])
    expect(sea).toEqual([{ x: 0, y: 0 }])
  })

  it('tries the marker right side before alternate city positions', () => {
    const candidates = getMapLabelCandidateOffsets(
      'city',
      { width: 40, height: 16 },
      12
    )

    expect(candidates[0]).toEqual({ x: 32, y: 0 })
    expect(candidates).toContainEqual({ x: 0, y: -20 })
    expect(candidates).toContainEqual({ x: -32, y: 0 })
  })
})

describe('world map label collision layout', () => {
  it('moves only city text while preserving its world and screen anchors', () => {
    const area = label('VALDRAN', 'continent', 3122.8, 7819.9)
    const city = label('Garasden', 'capital', 1929.6, 2746.2)
    const result = layoutMapLabels(
      [
        {
          label: area,
          anchor: { x: 160, y: 110 },
          textSize: { width: 70, height: 24 },
        },
        {
          label: city,
          anchor: { x: 100, y: 100 },
          textSize: { width: 60, height: 18 },
        },
      ],
      { zoomSpan: 8, viewport }
    )
    const placedArea = result[0]
    const placedCity = result[1]

    expect(placedArea.textOffset).toEqual({ x: 0, y: 0 })
    expect(placedCity.textOffset).toEqual({ x: 0, y: -21 })
    expect(placedCity.anchor).toEqual({ x: 100, y: 100 })
    expect(placedCity.label.x).toBe(1929.6)
    expect(placedCity.label.z).toBe(2746.2)
  })

  it('gives the capital first choice when point labels compete', () => {
    const result = layoutMapLabels(
      [
        {
          label: label('town', 'town'),
          anchor: { x: 100, y: 100 },
          textSize: { width: 50, height: 16 },
        },
        {
          label: label('capital', 'capital'),
          anchor: { x: 100, y: 100 },
          textSize: { width: 50, height: 16 },
        },
      ],
      { zoomSpan: 8, viewport }
    )
    const town = result[0]
    const capital = result[1]

    expect(capital.textOffset).toEqual({ x: 37, y: 0 })
    expect(town.textOffset).not.toEqual(capital.textOffset)
    expect(town.textVisible).toBe(true)
  })

  it('keeps a town name above a nearby dungeon at regional zoom', () => {
    const result = layoutMapLabels(
      [
        {
          label: label('Old Crypt', 'dungeon'),
          anchor: { x: 100, y: 97 },
          textSize: { width: 58, height: 17 },
        },
        {
          label: label('Aldermark', 'town'),
          anchor: { x: 100, y: 100 },
          textSize: { width: 62, height: 17 },
        },
      ],
      {
        zoomSpan: 8,
        viewport,
        collisionPadding: 5,
        markerGap: 11,
      }
    )

    expect(result).toHaveLength(2)
    expect(result[0].textVisible).toBe(false)
    expect(result[1].textVisible).toBe(true)
  })

  it('hides text when no collision-free candidate fits but retains the marker', () => {
    const result = layoutMapLabels(
      [
        {
          label: label('Edra', 'city', 5840, 240),
          anchor: { x: 20, y: 20 },
          textSize: { width: 80, height: 20 },
        },
      ],
      {
        zoomSpan: 8,
        viewport: { left: 0, top: 0, right: 40, bottom: 40 },
      }
    )[0]

    expect(result.textVisible).toBe(false)
    expect(result.anchor).toEqual({ x: 20, y: 20 })
    expect(result.textBounds).toBeNull()
  })

  it('keeps labels clear of player and party marker bounds', () => {
    const result = layoutMapLabels(
      [
        {
          label: label('Garasden', 'capital'),
          anchor: { x: 100, y: 100 },
          textSize: { width: 60, height: 18 },
        },
      ],
      {
        zoomSpan: 8,
        viewport,
        reservedBounds: [{ left: 108, top: 88, right: 190, bottom: 112 }],
      }
    )[0]

    expect(result.textOffset).toEqual({ x: -42, y: 0 })
  })

  it('does not collide a settlement name with its own marker', () => {
    const result = layoutMapLabels(
      [
        {
          label: label('Garasden', 'capital'),
          anchor: { x: 100, y: 100 },
          textSize: { width: 60, height: 18 },
        },
      ],
      {
        zoomSpan: 8,
        viewport,
        collisionPadding: 5,
        markerGap: 11,
      }
    )[0]

    expect(result.textVisible).toBe(true)
    expect(result.textOffset).toEqual({ x: 41, y: 0 })
  })

  it('normalizes area label visibility for the mobile zoom range', () => {
    const hidden = layoutMapLabels(
      [
        {
          label: label('VALDRAN', 'continent'),
          anchor: { x: 200, y: 120 },
        },
      ],
      { zoomSpan: 4, viewport }
    )
    const visible = layoutMapLabels(
      [
        {
          label: label('VALDRAN', 'continent'),
          anchor: { x: 200, y: 120 },
        },
      ],
      { zoomSpan: 4, areaZoomSpan: 8, viewport }
    )

    expect(hidden).toHaveLength(0)
    expect(visible[0].textVisible).toBe(true)
  })
})

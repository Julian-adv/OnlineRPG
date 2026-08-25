import { describe, expect, it } from 'vitest'
import {
  persistWorldMapView,
  resolveWorldMapView,
  type SavedWorldMapView,
} from './worldMapViewState'

const saved: SavedWorldMapView = {
  camX: 120,
  camZ: 240,
  zoomSpan: 16,
}

describe('world map view state', () => {
  it('follows the current player and default zoom after reset', () => {
    expect(
      resolveWorldMapView(
        saved,
        { followPlayerOnOpen: true, useDefaultZoomOnOpen: true },
        { camX: 900, camZ: 700, zoomSpan: 8 },
        32
      )
    ).toEqual({ camX: 900, camZ: 700, zoomSpan: 8 })
  })

  it('restores an interacted view and clamps it to the device zoom range', () => {
    expect(
      resolveWorldMapView(
        saved,
        { followPlayerOnOpen: false, useDefaultZoomOnOpen: false },
        { camX: 900, camZ: 700, zoomSpan: 2 },
        4
      )
    ).toEqual({ camX: 120, camZ: 240, zoomSpan: 4 })
  })

  it('keeps reset dimensions unset when the dialog is destroyed', () => {
    expect(
      persistWorldMapView(
        { camX: 120, camZ: 240, zoomSpan: 12 },
        { followPlayerOnOpen: true, useDefaultZoomOnOpen: true }
      )
    ).toEqual({ camX: null, camZ: null, zoomSpan: null })
  })

  it('saves dimensions after camera and zoom interactions', () => {
    expect(
      persistWorldMapView(
        { camX: 120, camZ: 240, zoomSpan: 12 },
        { followPlayerOnOpen: false, useDefaultZoomOnOpen: false }
      )
    ).toEqual({ camX: 120, camZ: 240, zoomSpan: 12 })
  })
})

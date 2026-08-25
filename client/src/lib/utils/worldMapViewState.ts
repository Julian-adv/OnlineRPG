export interface WorldMapView {
  camX: number
  camZ: number
  zoomSpan: number
}

export interface SavedWorldMapView {
  camX: number | null
  camZ: number | null
  zoomSpan: number | null
}

export interface WorldMapOpenPreferences {
  followPlayerOnOpen: boolean
  useDefaultZoomOnOpen: boolean
}

export function resolveWorldMapView(
  saved: SavedWorldMapView,
  preferences: WorldMapOpenPreferences,
  fallback: WorldMapView,
  maxZoomSpan: number
): WorldMapView {
  const savedCamX = saved.camX
  const savedCamZ = saved.camZ
  const savedZoom = saved.zoomSpan
  const restoreCamera =
    !preferences.followPlayerOnOpen && savedCamX !== null && savedCamZ !== null
  const restoreZoom = !preferences.useDefaultZoomOnOpen && savedZoom !== null

  return {
    camX: restoreCamera ? savedCamX : fallback.camX,
    camZ: restoreCamera ? savedCamZ : fallback.camZ,
    zoomSpan: restoreZoom
      ? Math.min(savedZoom, maxZoomSpan)
      : fallback.zoomSpan,
  }
}

export function persistWorldMapView(
  view: WorldMapView,
  preferences: WorldMapOpenPreferences
): SavedWorldMapView {
  return {
    camX: preferences.followPlayerOnOpen ? null : view.camX,
    camZ: preferences.followPlayerOnOpen ? null : view.camZ,
    zoomSpan: preferences.useDefaultZoomOnOpen ? null : view.zoomSpan,
  }
}

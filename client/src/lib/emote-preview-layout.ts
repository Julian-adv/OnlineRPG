/** Border-box width of the emote panel (180 content + 20 padding + 2 border). */
export const EMOTE_PANEL_W = 202
/** Width the preview box needs beside the panel (188 border box + 9 offset). */
export const EMOTE_PREVIEW_W = 197

export interface PreviewPlacement {
  side: 'left' | 'right'
  /** False when neither side has room — the preview must not render at all. */
  fits: boolean
}

/** Which side of the emote panel the preview box fits on, if any.
 *  `panelLeft` is the panel's border-box left edge in viewport px. */
export function previewPlacement(
  panelLeft: number,
  viewportWidth: number
): PreviewPlacement {
  const fitsLeft = panelLeft >= EMOTE_PREVIEW_W
  const fitsRight = panelLeft + EMOTE_PANEL_W + EMOTE_PREVIEW_W <= viewportWidth
  return { side: fitsLeft ? 'left' : 'right', fits: fitsLeft || fitsRight }
}

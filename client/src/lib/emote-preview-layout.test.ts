import { describe, expect, it } from 'vitest'
import { EMOTE_PANEL_W, previewPlacement } from './emote-preview-layout'

/** The panel's default CSS position is `right: 16px`. */
function defaultPanelLeft(viewportWidth: number): number {
  return viewportWidth - EMOTE_PANEL_W - 16
}

describe('previewPlacement', () => {
  it('hangs left of the default-placed panel on a desktop viewport', () => {
    expect(previewPlacement(defaultPanelLeft(1920), 1920)).toEqual({
      side: 'left',
      fits: true,
    })
  })

  it('does not render at all on a phone-width viewport', () => {
    // 375px iPhone portrait: neither side of the default panel has room.
    expect(previewPlacement(defaultPanelLeft(375), 375).fits).toBe(false)
    // The default panel's left side gains room at exactly 415px.
    expect(previewPlacement(defaultPanelLeft(414), 414).fits).toBe(false)
    expect(previewPlacement(defaultPanelLeft(415), 415).fits).toBe(true)
  })

  it('flips right when the panel is dragged against the left edge', () => {
    expect(previewPlacement(50, 1000)).toEqual({ side: 'right', fits: true })
  })

  it('refuses the dead band where neither side fits a dragged panel', () => {
    // 500px window, panel dragged to x=150: left needs 197, right needs 399.
    expect(previewPlacement(150, 500).fits).toBe(false)
  })

  it('keeps the left side the moment it has room', () => {
    expect(previewPlacement(197, 500)).toEqual({ side: 'left', fits: true })
  })
})

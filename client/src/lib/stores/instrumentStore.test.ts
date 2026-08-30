import { beforeEach, describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import {
  closeInstrumentPanel,
  instrumentPanelVisible,
  instrumentPressedNotes,
  openInstrumentPanel,
  setInstrumentNotePressed,
} from './instrumentStore'

beforeEach(() => closeInstrumentPanel())

describe('instrumentStore', () => {
  it('opens with a clean pressed-note set', () => {
    setInstrumentNotePressed(4, true)
    openInstrumentPanel()

    expect(get(instrumentPanelVisible)).toBe(true)
    expect([...get(instrumentPressedNotes)]).toEqual([])
  })

  it('tracks independent notes and clears them on close', () => {
    openInstrumentPanel()
    setInstrumentNotePressed(2, true)
    setInstrumentNotePressed(7, true)
    setInstrumentNotePressed(2, false)

    expect([...get(instrumentPressedNotes)]).toEqual([7])

    closeInstrumentPanel()
    expect(get(instrumentPanelVisible)).toBe(false)
    expect([...get(instrumentPressedNotes)]).toEqual([])
  })
})

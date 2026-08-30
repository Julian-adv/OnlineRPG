import { describe, expect, it } from 'vitest'
import { INSTRUMENT_NOTES, getInstrumentNote } from './instrumentNotes'

describe('instrumentNotes', () => {
  it('defines the three natural-note rows in keyboard order', () => {
    expect(INSTRUMENT_NOTES).toHaveLength(22)
    expect(INSTRUMENT_NOTES.map((note) => note.key).join('')).toBe(
      'ZXCVBNMASDFGHJQWERTYUI'
    )
    expect(INSTRUMENT_NOTES.map((note) => note.name)).toEqual([
      'C3',
      'D3',
      'E3',
      'F3',
      'G3',
      'A3',
      'B3',
      'C4',
      'D4',
      'E4',
      'F4',
      'G4',
      'A4',
      'B4',
      'C5',
      'D5',
      'E5',
      'F5',
      'G5',
      'A5',
      'B5',
      'C6',
    ])
  })

  it('uses twelve-tone equal temperament with A4 at 440 Hz', () => {
    expect(INSTRUMENT_NOTES[12].frequencyHz).toBe(440)
    expect(INSTRUMENT_NOTES[0].frequencyHz).toBeCloseTo(130.8128, 4)
    expect(INSTRUMENT_NOTES[21].frequencyHz).toBeCloseTo(1046.5023, 4)
  })

  it('keeps the measured one-shot length for every note', () => {
    expect(INSTRUMENT_NOTES.map((note) => note.durationSeconds)).toEqual([
      2.5, 3, 2, 2, 3.5, 3, 2.5, 3, 3, 3, 3, 3.5, 3, 3.5, 1.5, 2, 2, 2.824807,
      2, 1.5, 2, 1,
    ])
  })

  it('rejects fractional and out-of-range note indexes', () => {
    expect(getInstrumentNote(-1)).toBeUndefined()
    expect(getInstrumentNote(1.5)).toBeUndefined()
    expect(getInstrumentNote(22)).toBeUndefined()
  })
})

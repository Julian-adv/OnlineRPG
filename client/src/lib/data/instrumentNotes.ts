export type InstrumentNoteName =
  | 'C3'
  | 'D3'
  | 'E3'
  | 'F3'
  | 'G3'
  | 'A3'
  | 'B3'
  | 'C4'
  | 'D4'
  | 'E4'
  | 'F4'
  | 'G4'
  | 'A4'
  | 'B4'
  | 'C5'
  | 'D5'
  | 'E5'
  | 'F5'
  | 'G5'
  | 'A5'
  | 'B5'
  | 'C6'

export type InstrumentKeyCode =
  | 'KeyZ'
  | 'KeyX'
  | 'KeyC'
  | 'KeyV'
  | 'KeyB'
  | 'KeyN'
  | 'KeyM'
  | 'KeyA'
  | 'KeyS'
  | 'KeyD'
  | 'KeyF'
  | 'KeyG'
  | 'KeyH'
  | 'KeyJ'
  | 'KeyQ'
  | 'KeyW'
  | 'KeyE'
  | 'KeyR'
  | 'KeyT'
  | 'KeyY'
  | 'KeyU'
  | 'KeyI'

export interface InstrumentNote {
  index: number
  name: InstrumentNoteName
  key: string
  keyCode: InstrumentKeyCode
  frequencyHz: number
  durationSeconds: number
}

const NOTE_SPECS = [
  ['C3', 'KeyZ', 48, 2.5],
  ['D3', 'KeyX', 50, 3],
  ['E3', 'KeyC', 52, 2],
  ['F3', 'KeyV', 53, 2],
  ['G3', 'KeyB', 55, 3.5],
  ['A3', 'KeyN', 57, 3],
  ['B3', 'KeyM', 59, 2.5],
  ['C4', 'KeyA', 60, 3],
  ['D4', 'KeyS', 62, 3],
  ['E4', 'KeyD', 64, 3],
  ['F4', 'KeyF', 65, 3],
  ['G4', 'KeyG', 67, 3.5],
  ['A4', 'KeyH', 69, 3],
  ['B4', 'KeyJ', 71, 3.5],
  ['C5', 'KeyQ', 72, 1.5],
  ['D5', 'KeyW', 74, 2],
  ['E5', 'KeyE', 76, 2],
  ['F5', 'KeyR', 77, 2.824807],
  ['G5', 'KeyT', 79, 2],
  ['A5', 'KeyY', 81, 1.5],
  ['B5', 'KeyU', 83, 2],
  ['C6', 'KeyI', 84, 1],
] as const satisfies readonly (readonly [
  InstrumentNoteName,
  InstrumentKeyCode,
  number,
  number,
])[]

export const INSTRUMENT_NOTES: readonly InstrumentNote[] = NOTE_SPECS.map(
  ([name, keyCode, midi, durationSeconds], index) => ({
    index,
    name,
    key: keyCode.slice(3),
    keyCode,
    frequencyHz: 440 * 2 ** ((midi - 69) / 12),
    durationSeconds,
  })
)

export const INSTRUMENT_NOTE_COUNT = INSTRUMENT_NOTES.length

export function getInstrumentNote(index: number): InstrumentNote | undefined {
  return Number.isInteger(index) ? INSTRUMENT_NOTES[index] : undefined
}

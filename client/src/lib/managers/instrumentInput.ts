import { INSTRUMENT_NOTES, getInstrumentNote } from '../data/instrumentNotes'

export const INSTRUMENT_BATCH_MS = 250
// The server drops any batch above its cap, so flush early instead of
// letting a keyboard mash grow one past it.
export const INSTRUMENT_BATCH_MAX_EVENTS = 16

export const INSTRUMENT_NOTE_BY_CODE: ReadonlyMap<string, number> = new Map(
  INSTRUMENT_NOTES.map((note) => [note.keyCode, note.index])
)

export interface InstrumentNoteEvent {
  note: number
  offsetMs: number
}

export class InstrumentKeyLatch {
  private readonly pressed = new Set<string>()

  press(code: string, repeated = false): number | null {
    const note = INSTRUMENT_NOTE_BY_CODE.get(code)
    if (note === undefined || repeated || this.pressed.has(code)) return null
    this.pressed.add(code)
    return note
  }

  release(code: string) {
    this.pressed.delete(code)
  }

  clear() {
    this.pressed.clear()
  }
}

type TimerHandle = ReturnType<typeof setTimeout>

interface InstrumentNoteBatcherOptions {
  now?: () => number
  schedule?: (callback: () => void, delayMs: number) => TimerHandle
  cancel?: (timer: TimerHandle) => void
}

const defaultNow = () =>
  typeof performance === 'undefined' ? Date.now() : performance.now()

export class InstrumentNoteBatcher {
  private readonly now: () => number
  private readonly schedule: (
    callback: () => void,
    delayMs: number
  ) => TimerHandle
  private readonly cancel: (timer: TimerHandle) => void
  private readonly onFlush: (events: readonly InstrumentNoteEvent[]) => void
  private events: InstrumentNoteEvent[] = []
  private startedAt: number | null = null
  private timer: TimerHandle | null = null

  constructor(
    onFlush: (events: readonly InstrumentNoteEvent[]) => void,
    options: InstrumentNoteBatcherOptions = {}
  ) {
    this.onFlush = onFlush
    this.now = options.now ?? defaultNow
    this.schedule =
      options.schedule ?? ((callback, delay) => setTimeout(callback, delay))
    this.cancel = options.cancel ?? ((timer) => clearTimeout(timer))
  }

  get pendingCount(): number {
    return this.events.length
  }

  add(note: number, atMs = this.now()): boolean {
    if (!getInstrumentNote(note) || !Number.isFinite(atMs)) return false

    if (
      this.startedAt !== null &&
      atMs - this.startedAt >= INSTRUMENT_BATCH_MS
    ) {
      this.flush()
    }

    if (this.startedAt === null) {
      this.startedAt = atMs
      this.timer = this.schedule(() => this.flush(), INSTRUMENT_BATCH_MS)
    }

    this.events.push({
      note,
      offsetMs: Math.min(
        INSTRUMENT_BATCH_MS - 1,
        Math.max(0, Math.round(atMs - this.startedAt))
      ),
    })
    if (this.events.length >= INSTRUMENT_BATCH_MAX_EVENTS) this.flush()
    return true
  }

  flush(): InstrumentNoteEvent[] {
    if (this.timer !== null) this.cancel(this.timer)
    this.timer = null
    this.startedAt = null
    if (this.events.length === 0) return []

    const flushed = this.events
    this.events = []
    this.onFlush(flushed)
    return flushed
  }

  clear() {
    if (this.timer !== null) this.cancel(this.timer)
    this.timer = null
    this.startedAt = null
    this.events = []
  }

  dispose(flush = false) {
    if (flush) this.flush()
    else this.clear()
  }
}

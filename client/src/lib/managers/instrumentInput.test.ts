import { describe, expect, it, vi } from 'vitest'
import {
  INSTRUMENT_BATCH_MAX_EVENTS,
  INSTRUMENT_BATCH_MS,
  INSTRUMENT_NOTE_BY_CODE,
  InstrumentKeyLatch,
  InstrumentNoteBatcher,
} from './instrumentInput'

describe('instrument input', () => {
  it('maps all 22 physical keys to note indexes', () => {
    expect(INSTRUMENT_NOTE_BY_CODE.size).toBe(22)
    expect(INSTRUMENT_NOTE_BY_CODE.get('KeyZ')).toBe(0)
    expect(INSTRUMENT_NOTE_BY_CODE.get('KeyH')).toBe(12)
    expect(INSTRUMENT_NOTE_BY_CODE.get('KeyI')).toBe(21)
  })

  it('fires once per press and rearms on keyup', () => {
    const latch = new InstrumentKeyLatch()
    expect(latch.press('KeyA')).toBe(7)
    expect(latch.press('KeyA')).toBeNull()
    expect(latch.press('KeyA', true)).toBeNull()
    latch.release('KeyA')
    expect(latch.press('KeyA')).toBe(7)
  })

  it('batches relative timing and flushes after 250 ms', () => {
    let now = 1000
    let scheduled: (() => void) | null = null
    const onFlush = vi.fn()
    const batcher = new InstrumentNoteBatcher(onFlush, {
      now: () => now,
      schedule: (callback, delay) => {
        expect(delay).toBe(INSTRUMENT_BATCH_MS)
        scheduled = callback
        return 1 as unknown as ReturnType<typeof setTimeout>
      },
      cancel: () => {},
    })

    batcher.add(0)
    now += 37.4
    batcher.add(7)
    expect(batcher.pendingCount).toBe(2)
    ;(scheduled as (() => void) | null)?.()

    expect(onFlush).toHaveBeenCalledWith([
      { note: 0, offsetMs: 0 },
      { note: 7, offsetMs: 37 },
    ])
    expect(batcher.pendingCount).toBe(0)
  })

  it('starts a fresh batch if the timer was delayed', () => {
    let now = 500
    const flushed: unknown[] = []
    const batcher = new InstrumentNoteBatcher(
      (events) => flushed.push(events),
      {
        now: () => now,
        schedule: () => 1 as unknown as ReturnType<typeof setTimeout>,
        cancel: () => {},
      }
    )

    batcher.add(3)
    now += INSTRUMENT_BATCH_MS
    batcher.add(4)

    expect(flushed).toEqual([[{ note: 3, offsetMs: 0 }]])
    expect(batcher.flush()).toEqual([{ note: 4, offsetMs: 0 }])
  })

  it('keeps a rounded offset below the server batch limit', () => {
    let now = 1000
    const batcher = new InstrumentNoteBatcher(() => {}, {
      now: () => now,
      schedule: () => 1 as unknown as ReturnType<typeof setTimeout>,
      cancel: () => {},
    })

    batcher.add(0)
    now += INSTRUMENT_BATCH_MS - 0.1
    batcher.add(1)

    expect(batcher.flush()).toEqual([
      { note: 0, offsetMs: 0 },
      { note: 1, offsetMs: INSTRUMENT_BATCH_MS - 1 },
    ])
  })

  it('rejects invalid notes and can discard pending input', () => {
    const onFlush = vi.fn()
    const batcher = new InstrumentNoteBatcher(onFlush, {
      schedule: () => 1 as unknown as ReturnType<typeof setTimeout>,
      cancel: () => {},
    })

    expect(batcher.add(22)).toBe(false)
    expect(batcher.add(0)).toBe(true)
    batcher.dispose()
    expect(batcher.pendingCount).toBe(0)
    expect(onFlush).not.toHaveBeenCalled()
  })

  it('flushes early at the server batch cap instead of overflowing it', () => {
    const onFlush = vi.fn()
    const batcher = new InstrumentNoteBatcher(onFlush, {
      now: () => 1000,
      schedule: () => 1 as unknown as ReturnType<typeof setTimeout>,
      cancel: () => {},
    })

    for (let i = 0; i < INSTRUMENT_BATCH_MAX_EVENTS + 1; i++) {
      batcher.add(i % 22)
    }

    expect(onFlush).toHaveBeenCalledOnce()
    expect(onFlush.mock.calls[0][0]).toHaveLength(INSTRUMENT_BATCH_MAX_EVENTS)
    expect(batcher.pendingCount).toBe(1)
  })
})

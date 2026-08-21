import { describe, expect, it, vi } from 'vitest'
import {
  clearXpArrival,
  hasPendingXpArrival,
  queueXpArrival,
  releaseXpArrival,
  XP_ARRIVAL_TIMEOUT_MS,
} from './xpArrival'

describe('xpArrival', () => {
  it('holds the gain until it is released', () => {
    const arrive = vi.fn()
    queueXpArrival(
      { level: 3, totalXp: 120, lines: ['You gained 8 XP.'] },
      'goblin-1',
      arrive
    )
    expect(hasPendingXpArrival()).toBe(true)
    expect(arrive).not.toHaveBeenCalled()

    releaseXpArrival()
    expect(arrive).toHaveBeenCalledWith({
      level: 3,
      totalXp: 120,
      lines: ['You gained 8 XP.'],
    })
    expect(hasPendingXpArrival()).toBe(false)
  })

  it('keeps the highest total but every kill line', () => {
    const arrive = vi.fn()
    queueXpArrival({ level: 4, totalXp: 200, lines: ['second'] }, 'a', arrive)
    queueXpArrival({ level: 3, totalXp: 150, lines: ['first'] }, 'b', arrive)
    releaseXpArrival()
    expect(arrive).toHaveBeenCalledWith({
      level: 4,
      totalXp: 200,
      lines: ['second', 'first'],
    })
  })

  it('another monster falling does not release this kill', () => {
    const arrive = vi.fn()
    queueXpArrival({ level: 3, totalXp: 90, lines: [] }, 'goblin-1', arrive)
    releaseXpArrival('orc-7')
    expect(arrive).not.toHaveBeenCalled()
    releaseXpArrival('goblin-1')
    expect(arrive).toHaveBeenCalledTimes(1)
  })

  it('releasing twice does not repeat the gain', () => {
    const arrive = vi.fn()
    queueXpArrival({ level: 2, totalXp: 50, lines: [] }, 'a', arrive)
    releaseXpArrival()
    releaseXpArrival()
    expect(arrive).toHaveBeenCalledTimes(1)
  })

  it('clearing drops the held XP instead of showing it', () => {
    const arrive = vi.fn()
    queueXpArrival({ level: 6, totalXp: 400, lines: [] }, 'a', arrive)
    clearXpArrival()
    releaseXpArrival()
    expect(arrive).not.toHaveBeenCalled()
    expect(hasPendingXpArrival()).toBe(false)
  })

  it('a corpse that never finishes still lands the XP', () => {
    vi.useFakeTimers()
    const arrive = vi.fn()
    queueXpArrival({ level: 5, totalXp: 300, lines: [] }, 'a', arrive)
    vi.advanceTimersByTime(XP_ARRIVAL_TIMEOUT_MS)
    expect(arrive).toHaveBeenCalledWith({ level: 5, totalXp: 300, lines: [] })
    vi.useRealTimers()
  })
})

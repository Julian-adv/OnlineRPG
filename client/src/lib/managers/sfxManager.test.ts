import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cancelPendingFishingSounds, playFishingSound } from './sfxManager'

// sfxManager caches audio pools at module level, so FakeAudio instances
// survive across tests — count plays in a per-test map instead.
let plays = new Map<string, number>()

class FakeAudio {
  preload = ''
  volume = 1
  currentTime = 0
  constructor(public url: string) {}
  load() {}
  play() {
    plays.set(this.url, (plays.get(this.url) ?? 0) + 1)
    return Promise.resolve()
  }
}

function playedCount(urlPart: string) {
  let sum = 0
  for (const [url, count] of plays) {
    if (url.includes(urlPart)) sum += count
  }
  return sum
}

describe('delayed fishing sounds', () => {
  beforeEach(() => {
    plays = new Map()
    vi.useFakeTimers()
    vi.stubGlobal('Audio', FakeAudio)
    vi.stubGlobal('window', {
      setTimeout: setTimeout.bind(globalThis),
      clearTimeout: clearTimeout.bind(globalThis),
    })
  })

  afterEach(() => {
    cancelPendingFishingSounds()
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('a delayed splash plays once the flight time elapses', () => {
    playFishingSound('splash', 1400)

    expect(playedCount('splash')).toBe(0)
    vi.advanceTimersByTime(1400)
    expect(playedCount('splash')).toBe(1)
  })

  it('an aborted cast cancels the pending splash — no splash after the line is back in', () => {
    playFishingSound('splash', 1400)

    cancelPendingFishingSounds()

    vi.advanceTimersByTime(5000)
    expect(playedCount('splash')).toBe(0)
  })

  it('cancels every pending timer, whoosh included, when aborted inside the swing delay', () => {
    playFishingSound('cast', 200)
    playFishingSound('splash', 1400)

    cancelPendingFishingSounds()

    vi.advanceTimersByTime(5000)
    expect(playedCount('cast')).toBe(0)
    expect(playedCount('splash')).toBe(0)
  })

  it('cancel does not touch sounds that already played', () => {
    playFishingSound('splash', 1400)
    vi.advanceTimersByTime(1400)

    cancelPendingFishingSounds()

    expect(playedCount('splash')).toBe(1)
  })

  it('immediate sounds are unaffected by a pending-timer cancel', () => {
    cancelPendingFishingSounds()
    playFishingSound('plop')

    expect(playedCount('plop')).toBe(1)
  })
})

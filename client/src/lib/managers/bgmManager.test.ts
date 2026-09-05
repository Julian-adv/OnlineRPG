import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { BGM_TRACKS } from '../data/bgmTracks'

type Bgm = typeof import('./bgmManager')

class FakeAudio {
  static created: FakeAudio[] = []
  src = ''
  loop = false
  muted = false
  volume = 1
  paused = true
  ended = false
  dataset: Record<string, string> = {}
  play = vi.fn(() => {
    this.paused = false
    return Promise.resolve()
  })
  pause = vi.fn(() => {
    this.paused = true
  })
  addEventListener = vi.fn()
  removeEventListener = vi.fn()
  removeAttribute = vi.fn()
  load = vi.fn()
  constructor() {
    FakeAudio.created.push(this)
  }
}

let bgm: Bgm
let fetchMock: ReturnType<typeof vi.fn>

const flush = () => new Promise((r) => setTimeout(r, 0))

beforeEach(async () => {
  vi.resetModules()
  FakeAudio.created = []
  fetchMock = vi.fn(async () => ({
    ok: true,
    blob: async () => new Blob(['x']),
  }))
  vi.stubGlobal('fetch', fetchMock)
  vi.stubGlobal('Audio', FakeAudio)
  vi.stubGlobal('localStorage', undefined)
  let n = 0
  vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:test/${++n}`)
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
  bgm = await import('./bgmManager')
})

afterEach(() => {
  bgm.disposeBgm()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('bgmManager fetches tracks whole and plays them from a blob', () => {
  it('playlist track is fetched, then played from an object URL', async () => {
    bgm.startBgm()
    await flush()
    const el = FakeAudio.created[0]
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/bgm\//)
    expect(el.src).toBe('blob:test/1')
    expect(el.play).toHaveBeenCalled()
  })

  it('falls back to the direct URL when the fetch fails', async () => {
    fetchMock.mockRejectedValueOnce(new Error('offline'))
    bgm.startBgm()
    await flush()
    expect(FakeAudio.created[0].src).toMatch(/\/bgm\//)
    expect(FakeAudio.created[0].play).toHaveBeenCalled()
  })

  it('battle track is fetched into the one battle element and played', async () => {
    bgm.startBattleMusic()
    await flush()
    const battle = FakeAudio.created.find((a) => a.loop)!
    expect(battle.src).toBe('blob:test/1')
    expect(battle.play).toHaveBeenCalledTimes(1)
  })

  it('a fetch that finishes after battle music took over does not play', async () => {
    let resolveFetch!: (v: unknown) => void
    fetchMock.mockImplementationOnce(
      () => new Promise((r) => (resolveFetch = r))
    )
    bgm.startBgm()
    const playlistEl = FakeAudio.created[0]
    bgm.startBattleMusic()
    resolveFetch({ ok: true, blob: async () => new Blob(['x']) })
    await flush()
    expect(playlistEl.src).toBe('')
    expect(playlistEl.play).not.toHaveBeenCalled()
    // The battle blob resolved first (blob:test/1); the late playlist one is dropped.
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:test/2')
  })

  it('releases the finished track blob before the quiet gap', async () => {
    bgm.startBgm()
    await flush()
    const el = FakeAudio.created[0]
    const ended = el.addEventListener.mock.calls.find(
      (c) => c[0] === 'ended'
    )![1]
    ended()
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:test/1')
    expect(el.removeAttribute).toHaveBeenCalledWith('src')
  })

  it('disposes playlist, battle, and performance audio before module replacement', async () => {
    bgm.startBgm()
    await flush()
    bgm.playPerformance(BGM_TRACKS[0])
    await flush()
    bgm.startBattleMusic()
    await flush()
    const elements = [...FakeAudio.created]
    const sources = elements.map((el) => el.src)

    bgm.disposeBgm()

    for (const el of elements) {
      expect(el.paused).toBe(true)
      expect(el.removeAttribute).toHaveBeenCalledWith('src')
    }
    for (const src of sources) {
      expect(URL.revokeObjectURL).toHaveBeenCalledWith(src)
    }
    vi.resetModules()
    bgm = await import('./bgmManager')
    bgm.startBgm()
    await flush()
    expect(FakeAudio.created.filter((el) => !el.paused)).toHaveLength(1)
  })

  it.each(['startBgm', 'startBattleMusic'] as const)(
    'does not play a pending %s download after disposal',
    async (start) => {
      let resolveFetch!: (v: unknown) => void
      fetchMock.mockImplementationOnce(
        () => new Promise((resolve) => (resolveFetch = resolve))
      )
      bgm[start]()
      const el = FakeAudio.created[0]
      bgm.disposeBgm()
      resolveFetch({ ok: true, blob: async () => new Blob(['x']) })
      await flush()

      expect(el.play).not.toHaveBeenCalled()
      expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:test/1')
    }
  )

  it('cancels fades, delayed playback, and settings subscriptions on disposal', async () => {
    bgm.startBgm()
    await flush()
    bgm.startBattleMusic()
    await flush()
    vi.useFakeTimers()
    bgm.stopBattleMusic()
    bgm.holdLiveInstrumentQuiet()
    bgm.bgmVolume.set(0.5)
    const playCounts = FakeAudio.created.map((el) => el.play.mock.calls.length)

    bgm.disposeBgm()
    bgm.bgmVolume.set(0.8)
    bgm.bgmMuted.set(true)
    bgm.bgmMuted.set(false)
    expect(vi.getTimerCount()).toBe(0)
    await vi.runAllTimersAsync()
    expect(FakeAudio.created.map((el) => el.play.mock.calls.length)).toEqual(
      playCounts
    )
  })
})

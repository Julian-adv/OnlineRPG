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
  removeAttribute = vi.fn((name: string) => {
    if (name === 'src') this.src = ''
  })
  load = vi.fn()
  constructor() {
    FakeAudio.created.push(this)
  }
}

let bgm: Bgm
let fetchMock: ReturnType<typeof vi.fn>

const flush = () => new Promise((r) => setTimeout(r, 0))

function deferSuccessfulFetch() {
  let resolveFetch!: (value: unknown) => void
  fetchMock.mockImplementationOnce(
    () => new Promise((resolve) => (resolveFetch = resolve))
  )
  return () => resolveFetch({ ok: true, blob: async () => new Blob(['x']) })
}

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

  it('reuses battle blobs while each playback owns its object URL', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(0)
    bgm.startBattleMusic()
    await flush()
    const battle = FakeAudio.created[0]
    const firstSrc = battle.src

    bgm.stopBattleMusic()
    bgm.startBattleMusic()
    await flush()

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(battle.src).not.toBe(firstSrc)
    expect(URL.revokeObjectURL).toHaveBeenCalledWith(firstSrc)
    expect(battle.play).toHaveBeenCalledTimes(2)
  })

  it('shares an unfinished download across consecutive battles', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(0)
    const completeFetch = deferSuccessfulFetch()
    bgm.startBattleMusic()
    bgm.stopBattleMusic()
    bgm.startBattleMusic()
    expect(fetchMock).toHaveBeenCalledTimes(1)

    completeFetch()
    await flush()
    expect(FakeAudio.created[0].play).toHaveBeenCalledTimes(1)
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:test/1')
    expect(FakeAudio.created[0].src).toBe('blob:test/2')
  })

  it.each(['mute', 'zero volume'])(
    'defers playlist and battle downloads during %s, then starts the battle track',
    async (setting) => {
      if (setting === 'mute') bgm.bgmMuted.set(true)
      else bgm.bgmVolume.set(0)
      bgm.startBgm()
      bgm.startBattleMusic()
      await flush()
      expect(fetchMock).not.toHaveBeenCalled()
      const battle = FakeAudio.created[0]
      expect(battle.play).not.toHaveBeenCalled()

      if (setting === 'mute') bgm.bgmMuted.set(false)
      else bgm.bgmVolume.set(0.5)
      await flush()
      expect(fetchMock).toHaveBeenCalledTimes(1)
      expect(battle.src).toMatch(/^blob:/)
      expect(battle.play).toHaveBeenCalledTimes(1)
      expect(battle.muted).toBe(false)
    }
  )

  it('keeps downloads deferred when unmuted at zero volume', async () => {
    bgm.bgmMuted.set(true)
    bgm.bgmVolume.set(0)
    bgm.startBattleMusic()
    bgm.bgmMuted.set(false)
    await flush()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('retains a download completed while muted for unmute', async () => {
    const completeFetch = deferSuccessfulFetch()
    bgm.startBattleMusic()
    bgm.bgmMuted.set(true)
    completeFetch()
    await flush()
    const battle = FakeAudio.created[0]
    expect(battle.play).not.toHaveBeenCalled()

    bgm.bgmMuted.set(false)
    await flush()
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(battle.play).toHaveBeenCalledTimes(1)
  })

  it('loads the newly selected battle track after a muted battle transition', async () => {
    const random = vi.spyOn(Math, 'random').mockReturnValue(0)
    bgm.startBattleMusic()
    await flush()
    bgm.bgmMuted.set(true)
    bgm.stopBattleMusic()
    random.mockReturnValue(0.2)
    bgm.startBattleMusic()
    await flush()
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(FakeAudio.created[0].src).toBe('')

    bgm.bgmMuted.set(false)
    await flush()
    expect(fetchMock).toHaveBeenCalledTimes(2)
    expect(fetchMock.mock.calls[1][0]).not.toBe(fetchMock.mock.calls[0][0])
    expect(FakeAudio.created[0].dataset.trackName).toBe('Blood and Bronze (1)')
  })

  it.each(['network', 'http'])(
    'retries battle downloads after a %s failure',
    async (failure) => {
      vi.spyOn(Math, 'random').mockReturnValue(0)
      if (failure === 'network')
        fetchMock.mockRejectedValueOnce(new Error('offline'))
      else fetchMock.mockResolvedValueOnce({ ok: false })
      bgm.startBattleMusic()
      await flush()
      expect(FakeAudio.created[0].src).toMatch(/\/bgm\//)

      bgm.stopBattleMusic()
      bgm.startBattleMusic()
      await flush()
      expect(fetchMock).toHaveBeenCalledTimes(2)
      expect(FakeAudio.created[0].src).toMatch(/^blob:/)
    }
  )

  it('evicts old battle blobs to keep retained data within 48 MiB', async () => {
    const largeBlob = new Blob(['x'])
    Object.defineProperty(largeBlob, 'size', { value: 25 * 1024 * 1024 })
    fetchMock.mockResolvedValue({ ok: true, blob: async () => largeBlob })
    const random = vi.spyOn(Math, 'random').mockReturnValue(0)
    bgm.startBattleMusic()
    await flush()
    bgm.stopBattleMusic()
    random.mockReturnValue(0.2)
    bgm.startBattleMusic()
    await flush()
    bgm.stopBattleMusic()
    random.mockReturnValue(0)
    bgm.startBattleMusic()
    await flush()
    expect(fetchMock).toHaveBeenCalledTimes(3)
    expect(fetchMock.mock.calls[2][0]).toBe(fetchMock.mock.calls[0][0])
  })

  it('a fetch that finishes after battle music took over does not play', async () => {
    const completeFetch = deferSuccessfulFetch()
    bgm.startBgm()
    const playlistEl = FakeAudio.created[0]
    bgm.startBattleMusic()
    completeFetch()
    await flush()
    expect(playlistEl.src).toBe('')
    expect(playlistEl.play).not.toHaveBeenCalled()
    const battleSrc = FakeAudio.created.find((el) => el.loop)!.src
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1)
    expect(URL.revokeObjectURL).not.toHaveBeenCalledWith(battleSrc)
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
      const completeFetch = deferSuccessfulFetch()
      bgm[start]()
      const el = FakeAudio.created[0]
      bgm.disposeBgm()
      completeFetch()
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

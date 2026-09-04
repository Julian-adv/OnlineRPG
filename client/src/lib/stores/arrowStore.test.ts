import { beforeEach, describe, expect, it } from 'vitest'
import {
  ARROW_SPEED_MPS,
  arrowsInFlight,
  clearArrows,
  flightMsFor,
  landArrow,
  launchArrow,
  requestArrow,
  takeArrowRequests,
  type ArrowShot,
} from './arrowStore'
import { get } from 'svelte/store'

const shot = (monsterId = 'm1'): ArrowShot => ({
  monsterId,
  hit: true,
  from: { x: 0, y: 1, z: 0 },
  to: { x: 5, y: 1, z: 0 },
  flightMs: flightMsFor(5),
  launchedAt: 0,
})

beforeEach(() => clearArrows())

describe('flight time', () => {
  /** Fixed speed, not fixed duration: the delay before the damage number is
   *  what the arrow exists to explain, so it has to grow with the distance. */
  it('grows with distance at a fixed speed', () => {
    expect(flightMsFor(ARROW_SPEED_MPS)).toBeCloseTo(1000, 6)
    expect(flightMsFor(10)).toBeCloseTo((10 / ARROW_SPEED_MPS) * 1000, 6)
    expect(flightMsFor(2)).toBeLessThan(flightMsFor(10))
  })

  it('is zero at zero range rather than negative or infinite', () => {
    expect(flightMsFor(0)).toBe(0)
  })
})

describe('arrows in flight', () => {
  it('keeps one arrow per shooter, the later shot replacing the earlier', () => {
    launchArrow(7, shot('first'))
    launchArrow(7, shot('second'))
    const flights = get(arrowsInFlight)
    expect(flights.size).toBe(1)
    expect(flights.get(7)?.monsterId).toBe('second')
  })

  it('keeps separate shooters apart', () => {
    launchArrow(7, shot())
    launchArrow(9, shot())
    expect(get(arrowsInFlight).size).toBe(2)
    landArrow(7)
    expect([...get(arrowsInFlight).keys()]).toEqual([9])
  })

  /** The component calls this from its own frame loop, so a second call for
   *  an arrow already gone must not churn the store. */
  it('ignores landing an arrow that is already down', () => {
    launchArrow(7, shot())
    landArrow(7)
    const before = get(arrowsInFlight)
    landArrow(7)
    expect(get(arrowsInFlight)).toBe(before)
  })
})

describe('launch requests', () => {
  it('drains once, so a request cannot be fulfilled twice', () => {
    requestArrow({ playerId: 7, monsterId: 'm1', hit: true, flightMs: 100 })
    expect(takeArrowRequests()).toHaveLength(1)
    expect(takeArrowRequests()).toHaveLength(0)
  })

  /** Leaving the world drops shots that were mid-flight or still queued;
   *  neither should reappear in the next one. */
  it('is emptied along with the flights when the world goes', () => {
    launchArrow(7, shot())
    requestArrow({ playerId: 7, monsterId: 'm1', hit: true, flightMs: 100 })
    clearArrows()
    expect(get(arrowsInFlight).size).toBe(0)
    expect(takeArrowRequests()).toHaveLength(0)
  })
})

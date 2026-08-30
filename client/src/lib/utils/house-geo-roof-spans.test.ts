import { describe, expect, it } from 'vitest'
import { computeRoofSpans, roofSpanByRoom } from './house-geo-utils'
import type { RoomData, RoofType } from '../types/housing'
import { makeRoom as room } from './house-room.fixture'
import fixture from './__fixtures__/house-rooms.json'

/** Order-independent view of spans; rooms are reported as fixture indices. */
function describeSpans(rooms: RoomData[]) {
  return computeRoofSpans(rooms)
    .map((s) => ({
      rooms: s.rooms.map((r) => rooms.indexOf(r)).sort((a, b) => a - b),
      x: s.localX,
      z: s.localZ,
      sx: s.sizeX,
      sz: s.sizeZ,
      ridgeAlongX: s.ridgeAlongX,
      ridgeHeight: s.ridgeHeight,
      innerLow: s.innerLow,
      innerHigh: s.innerHigh,
      floor: s.rooms[0].floorLevel,
    }))
    .sort((a, b) => a.floor - b.floor || a.x - b.x || a.z - b.z || a.sx - b.sx)
}

/** Geometry-only view, ignoring which rooms back each span. */
function describeShapes(rooms: RoomData[]) {
  return describeSpans(rooms).map(({ rooms: _rooms, ...rest }) => rest)
}

describe('computeRoofSpans on live house data', () => {
  for (const house of fixture) {
    it(`matches the recorded spans for ${house.id}`, () => {
      const rooms = house.rooms.map((r) =>
        room({
          ...r,
          roomType: r.roomType as RoomData['roomType'],
        } as Partial<RoomData>)
      )
      expect(describeSpans(rooms)).toMatchSnapshot()
    })
  }
})

describe('computeRoofSpans', () => {
  const inn2F = (roofType: RoofType = 'steep') => [
    room({ localX: 0, localZ: 0, sizeX: 6, sizeZ: 4, floorLevel: 1, roofType }),
    room({ localX: 6, localZ: 0, sizeX: 6, sizeZ: 4, floorLevel: 1, roofType }),
    room({ localX: 0, localZ: 4, sizeX: 6, sizeZ: 4, floorLevel: 1, roofType }),
    room({ localX: 6, localZ: 4, sizeX: 6, sizeZ: 4, floorLevel: 1, roofType }),
  ]

  it('gives an interior partition the same roof as the unpartitioned floor', () => {
    const partitioned = [
      ...[0, 3, 6, 9].map((x) =>
        room({ localX: x, localZ: 0, sizeX: 3, sizeZ: 3, floorLevel: 1 })
      ),
      room({ localX: 0, localZ: 3, sizeX: 6, sizeZ: 2, floorLevel: 1 }),
      room({ localX: 6, localZ: 3, sizeX: 6, sizeZ: 2, floorLevel: 1 }),
      ...[0, 3, 6, 9].map((x) =>
        room({ localX: x, localZ: 5, sizeX: 3, sizeZ: 3, floorLevel: 1 })
      ),
    ]
    expect(describeShapes(partitioned)).toEqual(describeShapes(inn2F()))
  })

  it('keeps 1m corridors inside the footprint roof', () => {
    const rooms = [
      room({ localX: 0, localZ: 0, sizeX: 6, sizeZ: 3, floorLevel: 1 }),
      room({ localX: 6, localZ: 0, sizeX: 4, sizeZ: 6, floorLevel: 1 }),
      room({ localX: 10, localZ: 0, sizeX: 1, sizeZ: 8, floorLevel: 1 }),
      room({ localX: 11, localZ: 0, sizeX: 1, sizeZ: 8, floorLevel: 1 }),
      room({ localX: 0, localZ: 3, sizeX: 6, sizeZ: 1, floorLevel: 1 }),
      room({ localX: 0, localZ: 4, sizeX: 6, sizeZ: 4, floorLevel: 1 }),
      room({ localX: 6, localZ: 6, sizeX: 4, sizeZ: 2, floorLevel: 1 }),
    ]
    expect(describeShapes(rooms)).toEqual(describeShapes(inn2F()))
  })

  it('assigns every gabled room to exactly one span', () => {
    const rooms = inn2F('gabled')
    const byRoom = roofSpanByRoom(rooms)
    for (const r of rooms) expect(byRoom.get(r)).toBeDefined()
  })

  it('splits rooms with different roof types into separate spans', () => {
    const rooms = [
      room({ localX: 0, localZ: 0, sizeX: 4, sizeZ: 4, roofType: 'gabled' }),
      room({ localX: 4, localZ: 0, sizeX: 4, sizeZ: 4, roofType: 'steep' }),
    ]
    expect(describeSpans(rooms)).toHaveLength(2)
  })

  it('honours an explicit ridge direction over the footprint long axis', () => {
    const rooms = [
      room({ localX: 0, localZ: 0, sizeX: 6, sizeZ: 4, roofRidgeDir: 'z' }),
      room({ localX: 6, localZ: 0, sizeX: 6, sizeZ: 4, roofRidgeDir: 'z' }),
    ]
    for (const s of describeSpans(rooms)) expect(s.ridgeAlongX).toBe(false)
  })
})

describe('computeRoofSpans depth cap', () => {
  it('splits a deep footprint into an M-shape at a room boundary', () => {
    const rooms = [0, 6, 12].map((z) =>
      room({ localX: 0, localZ: z, sizeX: 6, sizeZ: 6, roofRidgeDir: 'x' })
    )
    const spans = describeSpans(rooms)
    expect(spans.map((s) => [s.z, s.sz, s.innerLow, s.innerHigh])).toEqual([
      [0, 6, false, true],
      [6, 12, true, false],
    ])
  })
})

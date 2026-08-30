import { describe, expect, it } from 'vitest'
import { facingSegmentRef } from './housing-passability'
import { makeRoom } from '../utils/house-room.fixture'

const room = (x: number, z: number, sx: number, sz: number) =>
  makeRoom({ localX: x, localZ: z, sizeX: sx, sizeZ: sz, floorLevel: 1 })

describe('facingSegmentRef', () => {
  const rooms = [room(0, 0, 3, 3), room(0, 3, 6, 2), room(3, 0, 3, 3)]

  it('maps a south segment onto the corridor segment beneath it', () => {
    expect(facingSegmentRef(rooms, 0, 'south', 1)).toEqual({
      roomIndex: 1,
      segmentIndex: 1,
    })
    expect(facingSegmentRef(rooms, 2, 'south', 0)).toEqual({
      roomIndex: 1,
      segmentIndex: 3,
    })
  })

  it('maps the corridor back onto the room above', () => {
    expect(facingSegmentRef(rooms, 1, 'north', 4)).toEqual({
      roomIndex: 2,
      segmentIndex: 1,
    })
  })

  it('returns null on exterior faces', () => {
    expect(facingSegmentRef(rooms, 0, 'north', 0)).toBeNull()
    expect(facingSegmentRef(rooms, 0, 'west', 2)).toBeNull()
  })

  it('ignores rooms on other floors', () => {
    const other = makeRoom({ localZ: 3, sizeX: 6, sizeZ: 2 })
    expect(facingSegmentRef([rooms[0], other], 0, 'south', 0)).toBeNull()
  })
})

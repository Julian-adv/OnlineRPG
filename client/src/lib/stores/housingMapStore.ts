import { writable } from 'svelte/store'
import type { HouseData, HouseMapFootprint } from '../types/housing'

export const houseMapFootprints = writable<HouseMapFootprint[]>([])

function houseMapFootprintFromHouse(
  house: HouseData
): HouseMapFootprint | null {
  const rects = house.rooms
    .filter((room) => room.floorLevel === 0 && room.roomType !== 'stairwell')
    .map(
      (room) =>
        [
          house.origin.x + room.localX,
          house.origin.z + room.localZ,
          room.sizeX,
          room.sizeZ,
        ] as [number, number, number, number]
    )
  return rects.length > 0 ? { rects } : null
}

export function setHouseMapFootprints(houses: HouseData[]) {
  houseMapFootprints.set(
    houses
      .map(houseMapFootprintFromHouse)
      .filter((entry): entry is HouseMapFootprint => entry !== null)
  )
}

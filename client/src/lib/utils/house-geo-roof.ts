/**
 * house-geo-roof.ts — Flat and gabled roof geometry generation.
 */
import * as THREE from 'three'
import type { RoomData, WallConfig } from '../types/housing'
import {
  WALL_THICKNESS,
  FLOOR_THICKNESS,
  ROOF_OVERHANG,
  FRAME_DEPTH,
  WOOD_TEXTURE_IDX,
  SHUTTER_PANEL_TEXTURE_IDX,
  HOUSING_TEXTURES,
  bakedGeo,
  cellInFootprint,
  floorYBase,
  floorOverhang,
  gabledRoofDims,
  type GeoEntry,
  type RoomFootprint,
} from './house-geo-utils'

const _roofMatrix = new THREE.Matrix4()

const GABLE_BEAM_HEIGHT = 0.12
const GABLE_WIN_W = 0.6
const GABLE_WIN_H = 0.7
const GABLE_WIN_FRAME = 0.06
const GABLE_WIN_MARGIN = 0.25

export function shouldSuppressRoof(
  room: RoomData,
  upperFloorFootprints: RoomFootprint[]
): boolean {
  if (upperFloorFootprints.length === 0) return false
  for (let x = room.localX; x < room.localX + room.sizeX; x++) {
    for (let z = room.localZ; z < room.localZ + room.sizeZ; z++) {
      if (!upperFloorFootprints.some((fp) => cellInFootprint(x, z, fp))) {
        return false
      }
    }
  }
  return true
}

/** Generate roof geometry for a room (flat or gabled). */
export function collectRoofGeometry(
  room: RoomData,
  frontTarget: GeoEntry[],
  backTarget?: GeoEntry[],
  allRooms?: RoomData[]
) {
  if (room.roofType && room.roofType !== 'flat') {
    collectGabledRoof(
      room,
      frontTarget,
      backTarget ?? frontTarget,
      allRooms ?? [room]
    )
  } else {
    collectFlatRoof(room, frontTarget)
  }
}

/** True when `other`'s wall line meets `room`'s gable end at `end`. */
function abutsAtEnd(
  room: RoomData,
  other: RoomData,
  ridgeAlongX: boolean,
  end: -1 | 1
): boolean {
  const aStart = ridgeAlongX ? room.localX : room.localZ
  const aSize = ridgeAlongX ? room.sizeX : room.sizeZ
  const bStart = ridgeAlongX ? room.localZ : room.localX
  const bSize = ridgeAlongX ? room.sizeZ : room.sizeX
  const oStartA = ridgeAlongX ? other.localX : other.localZ
  const oSizeA = ridgeAlongX ? other.sizeX : other.sizeZ
  const oStartB = ridgeAlongX ? other.localZ : other.localX
  const oSizeB = ridgeAlongX ? other.sizeZ : other.sizeX
  const myEdge = end === -1 ? aStart : aStart + aSize
  const oEdge = end === -1 ? oStartA + oSizeA : oStartA
  if (oEdge !== myEdge) return false
  return oStartB + oSizeB > bStart && oStartB < bStart + bSize
}

/**
 * If an adjacent gabled room with a perpendicular ridge meets this room's
 * gable end, return how far to extend the ridge so it reaches the other
 * room's ridge centerline. Otherwise 0.
 */
function gableExtension(
  room: RoomData,
  ridgeAlongX: boolean,
  end: -1 | 1,
  allRooms: RoomData[]
): number {
  for (const other of allRooms) {
    if (other === room) continue
    if (other.floorLevel !== room.floorLevel) continue
    if (!other.roofType || other.roofType === 'flat') continue
    if (gabledRoofDims(other).ridgeAlongX === ridgeAlongX) continue
    if (!abutsAtEnd(room, other, ridgeAlongX, end)) continue

    const oStartA = ridgeAlongX ? other.localX : other.localZ
    const oSizeA = ridgeAlongX ? other.sizeX : other.sizeZ
    const aStart = ridgeAlongX ? room.localX : room.localZ
    const aSize = ridgeAlongX ? room.sizeX : room.sizeZ
    const myEdge = end === -1 ? aStart : aStart + aSize
    return Math.abs(oStartA + oSizeA / 2 - myEdge)
  }
  return 0
}

/** True when another same-floor room shares this gable end's wall line. */
function gableEndAbuts(
  room: RoomData,
  ridgeAlongX: boolean,
  end: -1 | 1,
  allRooms: RoomData[]
): boolean {
  return allRooms.some(
    (o) =>
      o !== room &&
      o.floorLevel === room.floorLevel &&
      abutsAtEnd(room, o, ridgeAlongX, end)
  )
}

function collectFlatRoof(room: RoomData, target: GeoEntry[]) {
  const { localX, localZ, sizeX, sizeZ, wallHeight } = room
  const yBase = floorYBase(room.floorLevel, wallHeight)
  const roofIdx = room.roofTexture % HOUSING_TEXTURES.length
  const oh = floorOverhang(room.floorLevel)
  const totalW = sizeX + oh * 2
  const totalD = sizeZ + oh * 2
  const roofPlane = new THREE.PlaneGeometry(totalW, totalD)
  roofPlane.rotateX(-Math.PI / 2)
  target.push({
    geo: bakedGeo(
      roofPlane,
      localX + sizeX / 2,
      yBase + FLOOR_THICKNESS / 2 + wallHeight + 0.001,
      localZ + sizeZ / 2,
      0,
      totalW,
      totalD
    ),
    textureIndex: roofIdx,
  })
}

/**
 * Build a gabled (맞배지붕) roof:
 * - 2 sloped rectangular planes
 * - 2 triangular gable walls at each end
 * - ROOF_OVERHANG eaves on all sides
 */
function collectGabledRoof(
  room: RoomData,
  frontTarget: GeoEntry[],
  backTarget: GeoEntry[],
  allRooms: RoomData[]
) {
  const { localX, localZ, sizeX, sizeZ, wallHeight } = room
  const yBase = floorYBase(room.floorLevel, wallHeight)
  const wallTopY = yBase + FLOOR_THICKNESS / 2 + wallHeight
  const roofIdx = room.roofTexture % HOUSING_TEXTURES.length
  const { ridgeAlongX, shortDim, ridgeHeight } = gabledRoofDims(room)
  const flOh = floorOverhang(room.floorLevel)

  const cx = localX + sizeX / 2
  const cz = localZ + sizeZ / 2
  const oh = ROOF_OVERHANG

  // Expand roof base by floor overhang so it covers the expanded walls
  const halfShort = shortDim / 2 + flOh
  const halfLong = (ridgeAlongX ? sizeX / 2 : sizeZ / 2) + flOh

  const slopeAngle = Math.atan2(ridgeHeight, halfShort)
  const eaveDropY = (oh * ridgeHeight) / halfShort
  const slopeLen =
    ((halfShort + oh) * Math.sqrt(halfShort ** 2 + ridgeHeight ** 2)) /
    halfShort

  // If an adjacent room's perpendicular ridge meets a gable, extend our ridge
  // into the adjacent room to its ridge centerline instead of overhanging.
  const extLow = gableExtension(room, ridgeAlongX, -1, allRooms)
  const extHigh = gableExtension(room, ridgeAlongX, 1, allRooms)
  const endLow = extLow > 0 ? extLow : oh
  const endHigh = extHigh > 0 ? extHigh : oh
  const ridgeLen = halfLong * 2 + endLow + endHigh
  const ridgeShift = (endHigh - endLow) / 2

  const ridgeExt = (WALL_THICKNESS * ridgeHeight) / halfShort
  const totalSlopeLen = slopeLen + ridgeExt

  // Build two slope slabs
  for (const side of [-1, 1] as const) {
    const geo = new THREE.BoxGeometry(ridgeLen, WALL_THICKNESS, totalSlopeLen)

    const uv = geo.getAttribute('uv')
    for (let i = 0; i < uv.count; i++) {
      uv.setXY(i, uv.getX(i) * ridgeLen, uv.getY(i) * totalSlopeLen)
    }

    // Miter cut: pull inner (-Y) vertices at ridge end back toward eave
    const pos = geo.getAttribute('position')
    const innerY = -WALL_THICKNESS / 2
    const ridgeEndZ = (-side * totalSlopeLen) / 2
    for (let i = 0; i < pos.count; i++) {
      if (
        Math.abs(pos.getY(i) - innerY) < 0.001 &&
        Math.abs(pos.getZ(i) - ridgeEndZ) < 0.001
      ) {
        pos.setZ(i, ridgeEndZ + side * ridgeExt)
      }
    }

    geo.translate(0, WALL_THICKNESS / 2, (-side * ridgeExt) / 2)

    _roofMatrix.makeRotationX(side * slopeAngle)
    geo.applyMatrix4(_roofMatrix)

    if (!ridgeAlongX) {
      _roofMatrix.makeRotationY(Math.PI / 2)
      geo.applyMatrix4(_roofMatrix)
    }

    const perpCenter = (side * (halfShort + oh)) / 2
    const yCenter = wallTopY + (ridgeHeight - eaveDropY) / 2
    const tx = cx + (ridgeAlongX ? ridgeShift : perpCenter)
    const tz = cz + (ridgeAlongX ? perpCenter : ridgeShift)
    _roofMatrix.makeTranslation(tx, yCenter, tz)
    geo.applyMatrix4(_roofMatrix)

    frontTarget.push({ geo, textureIndex: roofIdx })
  }

  // Gable window fit check (same for both ends)
  const winHalfW = GABLE_WIN_W / 2
  const winCenterH = ridgeHeight * 0.38
  const winBot = winCenterH - GABLE_WIN_H / 2
  const winTop = winCenterH + GABLE_WIN_H / 2
  const edgeAtWinTop = halfShort * (1 - winTop / ridgeHeight)
  const hasWindow =
    edgeAtWinTop >= winHalfW + GABLE_WIN_MARGIN &&
    winTop + GABLE_WIN_MARGIN <= ridgeHeight &&
    winBot >= GABLE_WIN_MARGIN
  const edgeX = (y: number) => halfShort * (1 - y / ridgeHeight)

  for (const endSign of [-1, 1] as const) {
    // Extended ends merge into the adjacent room's roof — no gable triangle.
    const endExt = endSign === -1 ? extLow : extHigh
    if (endExt > 0) continue
    const endHasWindow =
      hasWindow && !gableEndAbuts(room, ridgeAlongX, endSign, allRooms)

    let wallSegs: WallConfig[]
    if (ridgeAlongX) {
      wallSegs = endSign === -1 ? room.wallWest : room.wallEast
    } else {
      wallSegs = endSign === -1 ? room.wallNorth : room.wallSouth
    }
    const gableTexIdx =
      (wallSegs.find((s) => s.variant !== 'open')?.texture ??
        room.roofTexture) % HOUSING_TEXTURES.length

    const endOffset = ridgeAlongX
      ? (endSign * (sizeX + flOh * 2)) / 2
      : (endSign * (sizeZ + flOh * 2)) / 2

    const gnx = ridgeAlongX ? endSign : 0
    const gnz = ridgeAlongX ? 0 : endSign
    const flipWinding = ridgeAlongX ? endSign === 1 : endSign === -1

    // Build vertex list: simple triangle or triangle with window cutout
    let localVerts: [number, number][]
    let indices: number[]
    if (!endHasWindow) {
      localVerts = [
        [-halfShort, 0],
        [halfShort, 0],
        [0, ridgeHeight],
      ]
      indices = flipWinding ? [0, 2, 1] : [0, 1, 2]
    } else {
      localVerts = [
        [-halfShort, 0], // 0: BL
        [halfShort, 0], // 1: BR
        [0, ridgeHeight], // 2: Peak
        [-edgeX(winBot), winBot], // 3: left edge @ winBot
        [-winHalfW, winBot], // 4: window BL
        [winHalfW, winBot], // 5: window BR
        [edgeX(winBot), winBot], // 6: right edge @ winBot
        [-edgeX(winTop), winTop], // 7: left edge @ winTop
        [-winHalfW, winTop], // 8: window TL
        [winHalfW, winTop], // 9: window TR
        [edgeX(winTop), winTop], // 10: right edge @ winTop
      ]
      // prettier-ignore
      const baseIdx = [
        0,1,6,  0,6,3,       // bottom strip
        3,4,8,  3,8,7,       // middle-left
        5,6,10, 5,10,9,      // middle-right
        7,8,2,  8,9,2, 9,10,2 // top strip
      ]
      indices = flipWinding ? reverseWinding(baseIdx) : baseIdx
    }

    // Populate geometry from local 2D verts
    const n = localVerts.length
    const positions = new Float32Array(n * 3)
    const normals = new Float32Array(n * 3)
    const uvs = new Float32Array(n * 2)
    for (let i = 0; i < n; i++) {
      const [perpOff, h] = localVerts[i]
      positions[i * 3] = cx + (ridgeAlongX ? endOffset : perpOff)
      positions[i * 3 + 1] = wallTopY + h
      positions[i * 3 + 2] = cz + (ridgeAlongX ? perpOff : endOffset)
      normals[i * 3] = gnx
      normals[i * 3 + 1] = 0
      normals[i * 3 + 2] = gnz
      uvs[i * 2] = perpOff + halfShort
      uvs[i * 2 + 1] = h
    }
    const geo = new THREE.BufferGeometry()
    geo.setIndex(indices)
    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('normal', new THREE.BufferAttribute(normals, 3))
    geo.setAttribute('uv', new THREE.BufferAttribute(uvs, 2))

    const isFront = ridgeAlongX ? endSign === -1 : endSign === 1
    const target = isFront ? frontTarget : backTarget
    target.push({ geo, textureIndex: gableTexIdx })

    if (WOOD_TEXTURE_IDX >= 0 && endHasWindow) {
      const faceX = cx + (ridgeAlongX ? endOffset : 0)
      const faceZ = cz + (ridgeAlongX ? 0 : endOffset)
      const beamRotY = ridgeAlongX ? Math.PI / 2 : 0

      // Base beam — front group so it hides with the roof when player is inside
      const beamWidth = halfShort * 2
      frontTarget.push({
        geo: bakedGeo(
          new THREE.BoxGeometry(beamWidth, GABLE_BEAM_HEIGHT, FRAME_DEPTH),
          faceX,
          wallTopY + GABLE_BEAM_HEIGHT / 2,
          faceZ,
          beamRotY,
          beamWidth,
          GABLE_BEAM_HEIGHT
        ),
        textureIndex: WOOD_TEXTURE_IDX,
      })

      const frameW = GABLE_WIN_W + GABLE_WIN_FRAME * 2
      for (const edgeH of [winBot, winTop]) {
        target.push({
          geo: bakedGeo(
            new THREE.BoxGeometry(frameW, GABLE_WIN_FRAME, FRAME_DEPTH),
            faceX,
            wallTopY + edgeH,
            faceZ,
            beamRotY,
            frameW,
            GABLE_WIN_FRAME
          ),
          textureIndex: WOOD_TEXTURE_IDX,
        })
      }
      const pillarH = GABLE_WIN_H + GABLE_WIN_FRAME * 2
      const winCenterY = wallTopY + winCenterH
      for (const sign of [-1, 1]) {
        const perpOff = sign * winHalfW
        target.push({
          geo: bakedGeo(
            new THREE.BoxGeometry(GABLE_WIN_FRAME, pillarH, FRAME_DEPTH),
            cx + (ridgeAlongX ? endOffset : perpOff),
            winCenterY,
            cz + (ridgeAlongX ? perpOff : endOffset),
            beamRotY,
            GABLE_WIN_FRAME,
            pillarH
          ),
          textureIndex: WOOD_TEXTURE_IDX,
        })
      }

      // Glass pane filling the window opening
      if (SHUTTER_PANEL_TEXTURE_IDX >= 0) {
        const glassGeo = new THREE.PlaneGeometry(GABLE_WIN_W, GABLE_WIN_H)
        if (flipWinding) glassGeo.scale(1, 1, -1)
        target.push({
          geo: bakedGeo(
            glassGeo,
            faceX,
            wallTopY + winCenterH,
            faceZ,
            beamRotY,
            1,
            1
          ),
          textureIndex: SHUTTER_PANEL_TEXTURE_IDX,
        })
      }
    }
  }
}

/** Swap v1 and v2 within each triangle to reverse face winding. */
function reverseWinding(indices: number[]): number[] {
  const out = new Array(indices.length)
  for (let i = 0; i < indices.length; i += 3) {
    out[i] = indices[i]
    out[i + 1] = indices[i + 2]
    out[i + 2] = indices[i + 1]
  }
  return out
}

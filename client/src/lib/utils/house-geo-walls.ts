/**
 * house-geo-walls.ts — Wall segment generation with door/window openings.
 */
import * as THREE from 'three'
import type { RoomData, WallConfig } from '../types/housing'
import { getHousingMaterial } from './housing-textures'
import {
  crossRoomDoorPartner,
  doubleDoorPartner,
  getWallByDir,
  isDoorVariant,
} from '../managers/housing-passability'
import {
  WALL_THICKNESS,
  FLOOR_THICKNESS,
  HOUSING_TEXTURES,
  FRAME_DEPTH,
  WOOD_TEXTURE_IDX,
  SHUTTER_PANEL_TEXTURE_IDX,
  WALL_DIR_INFO,
  bakedGeo,
  floorYBase,
  floorOverhang,
  type WallDirection,
  type GeoEntry,
  type DoorMeshInfo,
} from './house-geo-utils'

const DOOR_TEXTURE_IDX = WOOD_TEXTURE_IDX
const DOOR_WIDTH = 0.8
const DOOR_HEIGHT = 2.2
const WINDOW_WIDTH = 0.8
const WINDOW_HEIGHT = 1.0
const WINDOW_BOTTOM = 1.2
const FRAME_BEAM_FRAC = 0.05 // beam thickness as fraction of wall height
const FRAME_BEAM_Y_FRAC = 0.4 // beam position from bottom
const FRAME_SIDE_FRAC = 0.1 // side strip width as fraction of segment width
const FRAME_BOTTOM_FRAC = 0.05 // bottom strip height fraction
const FRAME_DIAG_THICKNESS = 0.06 // diagonal beam thickness in meters
const SHUTTER_BORDER = 0.045 // shutter border frame thickness (used for side-face UV)

// Shared temp matrix for frame diagonal geometry (single-threaded, sync only)
const _frameMat = new THREE.Matrix4()
const _frameTmp = new THREE.Matrix4()

/** Add side pillars, skipping corners where a corner pillar replaces them. */
function addFramePillars(
  target: GeoEntry[],
  x: number,
  yBase: number,
  z: number,
  rotY: number,
  segW: number,
  wh: number,
  isNS: boolean,
  skipLeft: boolean,
  skipRight: boolean,
  texIdx: number
) {
  const sideW = segW * FRAME_SIDE_FRAC
  for (const sign of [-1, 1]) {
    if (sign === -1 && skipLeft) continue
    if (sign === 1 && skipRight) continue
    const offset = sign * (segW / 2 - sideW / 2)
    target.push({
      geo: bakedGeo(
        new THREE.BoxGeometry(sideW, wh, FRAME_DEPTH),
        isNS ? x + offset : x,
        yBase + wh / 2,
        !isNS ? z + offset : z,
        rotY,
        sideW,
        wh
      ),
      textureIndex: texIdx,
    })
  }
}

/** Add X diagonals spanning an area of width×height. */
function addFrameXDiagonals(
  target: GeoEntry[],
  x: number,
  yBase: number,
  z: number,
  rotY: number,
  innerW: number,
  diagBottom: number,
  diagTop: number,
  texIdx: number
) {
  const diagH = diagTop - diagBottom
  if (diagH <= 0 || innerW <= 0) return

  const diagLen = Math.sqrt(innerW * innerW + diagH * diagH)
  const diagAngle = Math.atan2(diagH, innerW)
  const diagCenterY = yBase + diagBottom + diagH / 2

  for (const flipSign of [-1, 1]) {
    const geo = new THREE.BoxGeometry(
      diagLen,
      FRAME_DIAG_THICKNESS,
      FRAME_DEPTH
    )

    // Combine rotZ (diagonal tilt) + rotY + translate into one matrix
    _frameMat.makeRotationZ(flipSign * diagAngle)
    if (rotY !== 0) {
      _frameTmp.makeRotationY(rotY)
      _frameTmp.setPosition(x, diagCenterY, z)
      _frameMat.premultiply(_frameTmp)
    } else {
      _frameMat.setPosition(x, diagCenterY, z)
    }
    geo.applyMatrix4(_frameMat)

    const uv = geo.getAttribute('uv')
    if (uv) {
      for (let j = 0; j < uv.count; j++) {
        uv.setXY(j, uv.getX(j) * diagLen, uv.getY(j) * FRAME_DIAG_THICKNESS)
      }
    }

    target.push({ geo, textureIndex: texIdx })
  }
}

/** Add half-timber frame for a window wall. */
function addWindowFrameGeometry(
  target: GeoEntry[],
  x: number,
  yBase: number,
  z: number,
  rotY: number,
  segW: number,
  wh: number,
  openH: number,
  openBot: number,
  isNS: boolean,
  skipLeft: boolean,
  skipRight: boolean,
  woodTexIdx: number
) {
  const sideW = segW * FRAME_SIDE_FRAC
  const bottomH = wh * FRAME_BOTTOM_FRAC
  const innerW = segW - sideW * 2
  const beamH = wh * FRAME_BEAM_FRAC
  const beamY = wh * FRAME_BEAM_Y_FRAC

  addFramePillars(
    target,
    x,
    yBase,
    z,
    rotY,
    segW,
    wh,
    isNS,
    skipLeft,
    skipRight,
    woodTexIdx
  )

  // Bottom strip
  target.push({
    geo: bakedGeo(
      new THREE.BoxGeometry(segW, bottomH, FRAME_DEPTH),
      x,
      yBase + bottomH / 2,
      z,
      rotY,
      segW,
      bottomH
    ),
    textureIndex: woodTexIdx,
  })

  // Window header
  target.push({
    geo: bakedGeo(
      new THREE.BoxGeometry(innerW, FRAME_DIAG_THICKNESS, FRAME_DEPTH),
      x,
      yBase + openBot + openH,
      z,
      rotY,
      innerW,
      FRAME_DIAG_THICKNESS
    ),
    textureIndex: woodTexIdx,
  })

  // Horizontal beam above X area
  target.push({
    geo: bakedGeo(
      new THREE.BoxGeometry(innerW, beamH, FRAME_DEPTH),
      x,
      yBase + beamY,
      z,
      rotY,
      innerW,
      beamH
    ),
    textureIndex: woodTexIdx,
  })

  addFrameXDiagonals(
    target,
    x,
    yBase,
    z,
    rotY,
    innerW,
    bottomH,
    beamY - beamH / 2,
    woodTexIdx
  )
}

/** Add half-timber frame for a solid wall. */
function addFrameGeometry(
  target: GeoEntry[],
  x: number,
  yBase: number,
  z: number,
  rotY: number,
  segW: number,
  wh: number,
  isNS: boolean,
  skipLeft: boolean,
  skipRight: boolean,
  woodTexIdx: number
) {
  const beamH = wh * FRAME_BEAM_FRAC
  const beamY = wh * FRAME_BEAM_Y_FRAC
  const sideW = segW * FRAME_SIDE_FRAC
  const innerW = segW - sideW * 2
  const bottomH = wh * FRAME_BOTTOM_FRAC

  // Horizontal beam
  target.push({
    geo: bakedGeo(
      new THREE.BoxGeometry(segW, beamH, FRAME_DEPTH),
      x,
      yBase + beamY,
      z,
      rotY,
      segW,
      beamH
    ),
    textureIndex: woodTexIdx,
  })

  addFramePillars(
    target,
    x,
    yBase,
    z,
    rotY,
    segW,
    wh,
    isNS,
    skipLeft,
    skipRight,
    woodTexIdx
  )

  // Bottom strip
  target.push({
    geo: bakedGeo(
      new THREE.BoxGeometry(segW, bottomH, FRAME_DEPTH),
      x,
      yBase + bottomH / 2,
      z,
      rotY,
      segW,
      bottomH
    ),
    textureIndex: woodTexIdx,
  })

  addFrameXDiagonals(
    target,
    x,
    yBase,
    z,
    rotY,
    innerW,
    bottomH,
    beamY - beamH / 2,
    woodTexIdx
  )
}

/** Render 1m wall segments along a wall direction. */
export function collectWallSegments(
  segments: WallConfig[],
  dir: WallDirection,
  room: RoomData,
  roomIndex: number,
  frontEntries: GeoEntry[],
  backEntries: GeoEntry[],
  doors: DoorMeshInfo[],
  allRooms: RoomData[]
) {
  const dirInfo = WALL_DIR_INFO[dir]
  const target = dirInfo.isFront ? frontEntries : backEntries
  const wh = room.wallHeight
  const yBase = floorYBase(room.floorLevel, wh) + FLOOR_THICKNESS / 2
  const { localX, localZ, sizeX, sizeZ } = room
  const oh = floorOverhang(room.floorLevel)

  // Wall span: shrink by WALL_THICKNESS to avoid overlap at corners
  const halfT = WALL_THICKNESS / 2
  const numSegs = segments.length
  const wallSpan = (dirInfo.isNS ? sizeX : sizeZ) + oh * 2 - WALL_THICKNESS
  const segW = numSegs > 0 ? wallSpan / numSegs : 1

  for (let i = 0; i < segments.length; i++) {
    const seg = segments[i]
    if (seg.variant === 'open') continue

    // Double doors: the leader renders both segments; a lone half paired across
    // the room boundary opens through that side, otherwise it is a plain door
    let variant = seg.variant
    let span = 1
    let boundary: -1 | 0 | 1 = 0
    if (variant === 'double-door') {
      const partner = doubleDoorPartner(segments, i)
      if (partner >= 0 && partner === i - 1) continue
      if (partner >= 0) span = 2
      else {
        variant = 'door'
        if (crossRoomDoorPartner(allRooms, roomIndex, dir, i))
          boundary = i === 0 ? -1 : 1
      }
    }
    const W = segW * span
    const isDoor = isDoorVariant(variant)
    const lastIdx = i + span - 1

    const texIdx = seg.texture % HOUSING_TEXTURES.length

    // Position: offset by halfT to center within the shortened span
    const segCenter = i * segW + W / 2
    let x: number, z: number, rotY: number

    switch (dir) {
      case 'north': {
        x = localX - oh + halfT + segCenter
        z = localZ - oh + halfT
        rotY = 0
        break
      }
      case 'south': {
        x = localX - oh + halfT + segCenter
        z = localZ + sizeZ + oh - halfT
        rotY = 0
        break
      }
      case 'east': {
        x = localX + sizeX + oh - halfT
        z = localZ - oh + halfT + segCenter
        rotY = Math.PI / 2
        break
      }
      case 'west': {
        x = localX - oh + halfT
        z = localZ - oh + halfT + segCenter
        rotY = Math.PI / 2
        break
      }
    }

    // fitSegment: normalize UV to 0→1 per segment instead of world-space tiling
    const fit = HOUSING_TEXTURES[texIdx].fitSegment
    const u = (v: number) => (fit ? v / W : v)
    const v = (h: number) => (fit ? h / wh : h)

    if (variant === 'solid') {
      const wallH = fit ? wh - 0.01 : wh
      target.push({
        geo: bakedGeo(
          new THREE.BoxGeometry(segW, wallH, WALL_THICKNESS),
          x,
          yBase + wallH / 2,
          z,
          rotY,
          u(segW),
          v(wallH)
        ),
        textureIndex: texIdx,
      })
      if (fit && DOOR_TEXTURE_IDX >= 0) {
        const skipL = i === 0
        const skipR = i === segments.length - 1
        addFrameGeometry(
          target,
          x,
          yBase,
          z,
          rotY,
          segW,
          wh,
          dirInfo.isNS,
          skipL,
          skipR,
          DOOR_TEXTURE_IDX
        )
      }
    } else {
      // Opening = segment span minus a jamb per side; a boundary side has a
      // negative jamb so the opening runs through the shared wall end
      const sideW = isDoor ? (segW - DOOR_WIDTH) / 2 : (W - WINDOW_WIDTH) / 2
      const jambL = boundary === -1 ? -halfT : sideW
      const jambR = boundary === 1 ? -halfT : sideW
      const openW = W - jambL - jambR
      const openOff = (jambL - jambR) / 2
      const openH = isDoor ? DOOR_HEIGHT : WINDOW_HEIGHT
      const openBot = isDoor ? 0 : WINDOW_BOTTOM
      const ox = dirInfo.isNS ? x + openOff : x
      const oz = dirInfo.isNS ? z : z + openOff

      // Left and right solid strips (skip for fitSegment — frame pillars cover them)
      if (sideW > 0.01 && !fit) {
        for (const sign of [-1, 1]) {
          if (sign === boundary) continue
          const offset = sign * (W / 2 - sideW / 2)
          const sx = dirInfo.isNS ? x + offset : x
          const sz = !dirInfo.isNS ? z + offset : z
          const uOffX = sign === -1 ? 0 : W - sideW
          target.push({
            geo: bakedGeo(
              new THREE.BoxGeometry(sideW, wh, WALL_THICKNESS),
              sx,
              yBase + wh / 2,
              sz,
              rotY,
              u(sideW),
              v(wh),
              u(uOffX),
              0
            ),
            textureIndex: texIdx,
          })
        }
      }

      // Bottom strip (windows)
      if (openBot > 0.01) {
        target.push({
          geo: bakedGeo(
            new THREE.BoxGeometry(openW, openBot, WALL_THICKNESS),
            ox,
            yBase + openBot / 2,
            oz,
            rotY,
            u(openW),
            v(openBot),
            u(sideW),
            0
          ),
          textureIndex: texIdx,
        })
      }

      // Top strip
      const topH = wh - openBot - openH
      if (topH > 0.01) {
        target.push({
          geo: bakedGeo(
            new THREE.BoxGeometry(openW, topH, WALL_THICKNESS),
            ox,
            yBase + openBot + openH + topH / 2,
            oz,
            rotY,
            u(openW),
            v(topH),
            u(sideW),
            v(openBot + openH)
          ),
          textureIndex: texIdx,
        })
      }

      // Window frame geometry
      if (fit && variant === 'window' && DOOR_TEXTURE_IDX >= 0) {
        addWindowFrameGeometry(
          target,
          x,
          yBase,
          z,
          rotY,
          segW,
          wh,
          openH,
          openBot,
          dirInfo.isNS,
          i === 0,
          i === segments.length - 1,
          DOOR_TEXTURE_IDX
        )
      }

      // Door frame: pillars + header beam
      if (fit && isDoor && DOOR_TEXTURE_IDX >= 0) {
        const pillarW = W * FRAME_SIDE_FRAC
        const pillarL = boundary === -1 ? -halfT : pillarW
        const pillarR = boundary === 1 ? -halfT : pillarW
        const innerW = W - pillarL - pillarR
        const beamOff = (pillarL - pillarR) / 2
        addFramePillars(
          target,
          x,
          yBase,
          z,
          rotY,
          W,
          wh,
          dirInfo.isNS,
          i === 0 || boundary === -1,
          lastIdx === segments.length - 1 || boundary === 1,
          DOOR_TEXTURE_IDX
        )
        target.push({
          geo: bakedGeo(
            new THREE.BoxGeometry(innerW, FRAME_DIAG_THICKNESS, FRAME_DEPTH),
            dirInfo.isNS ? x + beamOff : x,
            yBase + DOOR_HEIGHT,
            dirInfo.isNS ? z : z + beamOff,
            rotY,
            innerW,
            FRAME_DIAG_THICKNESS
          ),
          textureIndex: DOOR_TEXTURE_IDX,
        })
      }

      // Shared panel setup for door / window hinged panels
      if (isDoor || variant === 'window') {
        const panelMat = getHousingMaterial(
          DOOR_TEXTURE_IDX >= 0 ? DOOR_TEXTURE_IDX : texIdx
        )
        const isOpen = seg.isOpen ?? false
        // Inset hinge to clear frame pillar
        const inset = Math.max(0, W * FRAME_SIDE_FRAC - sideW)

        if (isDoor) {
          const doorPanelH = DOOR_HEIGHT - FRAME_DIAG_THICKNESS / 2
          const leafW = span === 2 ? openW / 2 : openW
          const panelGeo = new THREE.BoxGeometry(
            leafW,
            doorPanelH,
            WALL_THICKNESS
          )
          // Interior-face offset
          const panelZ = (dirInfo.isFront ? -1 : 1) * (WALL_THICKNESS / 4)
          // E/W closes at -PI/2: panel local +X -> world +Z fills the opening
          const closedAngle = dirInfo.isNS ? 0 : -Math.PI / 2

          // Single door hinges on the low side (away from a boundary);
          // double doors add a mirrored leaf
          const hinges: (-1 | 1)[] =
            span === 2 ? [-1, 1] : boundary === -1 ? [1] : [-1]
          for (const h of hinges) {
            const segIdx = h === -1 ? i : lastIdx
            const panel = new THREE.Mesh(panelGeo, panelMat)
            panel.position.set(-h * (leafW / 2 - inset), doorPanelH / 2, panelZ)

            const pivot = new THREE.Group()
            pivot.name = `door_r${roomIndex}_${dir}_${segIdx}`

            const hingeOffset = openOff + h * (openW / 2 - inset)
            const openAngle = closedAngle + (h * Math.PI) / 2
            if (dirInfo.isNS) {
              pivot.position.set(x + hingeOffset, yBase, z)
            } else {
              pivot.position.set(x, yBase, z + hingeOffset)
            }
            pivot.rotation.y = isOpen ? openAngle : closedAngle
            pivot.add(panel)

            doors.push({
              pivot,
              roomIndex,
              wallDir: dir,
              segmentIndex: segIdx,
              floorLevel: room.floorLevel,
              isOpen,
              closedAngle,
              openAngle,
            })
          }
        } else {
          // Two hinged shutters per window: single box with composite texture
          // Interior-face Z offset
          const panelZ =
            (dirInfo.isFront === dirInfo.isNS ? -1 : 1) * (WALL_THICKNESS / 4)
          const closedAngle = dirInfo.isNS ? 0 : Math.PI / 2
          const halfW = WINDOW_WIDTH / 2
          const outwardSign = dirInfo.isFront ? 1 : -1

          // Panel height trimmed to fit between frame beam top and header bottom
          const beamTop = wh * FRAME_BEAM_Y_FRAC + (wh * FRAME_BEAM_FRAC) / 2
          const headerBot =
            WINDOW_BOTTOM + WINDOW_HEIGHT - FRAME_DIAG_THICKNESS / 2
          const panelH = headerBot - beamTop
          const panelYOff = beamTop - WINDOW_BOTTOM + panelH / 2

          const shutterGeo = new THREE.BoxGeometry(
            halfW,
            panelH,
            WALL_THICKNESS / 4
          )
          // Remap side face UVs to wood border region of composite texture
          {
            const uv = shutterGeo.getAttribute('uv')
            const woodU = SHUTTER_BORDER / halfW / 2
            const woodV = 0.5
            // Vertices 0–15 are the 4 side faces (+X, -X, +Y, -Y)
            for (let vi = 0; vi < 16; vi++) {
              uv.setXY(vi, woodU, woodV)
            }
          }
          const shutterMat = getHousingMaterial(SHUTTER_PANEL_TEXTURE_IDX)

          for (const side of [-1, 1] as const) {
            const panelX = (dirInfo.isNS ? -side : side) * (halfW / 2 - inset)

            const shutter = new THREE.Mesh(shutterGeo, shutterMat)
            shutter.position.set(panelX, panelYOff, panelZ)

            const pivot = new THREE.Group()
            pivot.name = `win_r${roomIndex}_${dir}_${i}_${side < 0 ? 'L' : 'R'}`

            const hingeOffset = side * (WINDOW_WIDTH / 2 - inset)
            const openAngle = closedAngle + outwardSign * side * (Math.PI / 2)

            if (dirInfo.isNS) {
              pivot.position.set(x + hingeOffset, yBase + WINDOW_BOTTOM, z)
            } else {
              pivot.position.set(x, yBase + WINDOW_BOTTOM, z + hingeOffset)
            }
            pivot.rotation.y = isOpen ? openAngle : closedAngle

            pivot.add(shutter)

            doors.push({
              pivot,
              roomIndex,
              wallDir: dir,
              segmentIndex: i,
              floorLevel: room.floorLevel,
              isOpen,
              closedAngle,
              openAngle,
            })
          }
        }
      }
    }
  }

  // Corner pillars: centered at wall center-line intersection, sized to FRAME_DEPTH
  // Only NS walls draw them to avoid doubles
  if (DOOR_TEXTURE_IDX >= 0 && numSegs > 0 && dirInfo.isNS) {
    const firstFit =
      HOUSING_TEXTURES[segments[0].texture % HOUSING_TEXTURES.length].fitSegment
    const lastFit =
      HOUSING_TEXTURES[segments[numSegs - 1].texture % HOUSING_TEXTURES.length]
        .fitSegment
    const cornerSize = FRAME_DEPTH

    for (const end of [0, 1] as const) {
      const isFit = end === 0 ? firstFit : lastFit
      if (!isFit) continue
      // No pillar where a boundary half-door meets the corner
      const endSeg = end === 0 ? 0 : numSegs - 1
      const perpDir: WallDirection = end === 0 ? 'west' : 'east'
      const perpSeg =
        dir === 'north' ? 0 : getWallByDir(room, perpDir).length - 1
      if (
        crossRoomDoorPartner(allRooms, roomIndex, dir, endSeg) ||
        crossRoomDoorPartner(allRooms, roomIndex, perpDir, perpSeg)
      )
        continue

      // Place at the intersection of this wall's center and the perpendicular wall's center
      const sign = end === 0 ? -1 : 1
      const alongOffset = (sign * wallSpan) / 2

      const cx = localX - oh + halfT + wallSpan / 2 + alongOffset
      const cz =
        dir === 'north' ? localZ - oh + halfT : localZ + sizeZ + oh - halfT

      target.push({
        geo: bakedGeo(
          new THREE.BoxGeometry(cornerSize, wh, cornerSize),
          cx,
          yBase + wh / 2,
          cz,
          0,
          cornerSize,
          wh
        ),
        textureIndex: DOOR_TEXTURE_IDX,
      })
    }
  }
}

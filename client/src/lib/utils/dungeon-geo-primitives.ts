/**
 * dungeon-geo-primitives.ts — generic mesh primitives for the dungeon builders:
 * a UV-baked box, a quad-mesh accumulator and a gabled roof. All push GeoEntry
 * slabs for the caller to merge; none knows about dungeon textures (callers
 * pass the texture index).
 */
import * as THREE from 'three'
import { bakedGeo, type GeoEntry } from './house-geo-utils'

/** Box with housing-style face UVs derived from final (baked) position.
 *  `uvScale` <1 enlarges the texture pattern (fewer repeats); callers pass it
 *  for the floor/stairs, everything else tiles 1:1 in metres. */
export function addBox(
  entries: GeoEntry[],
  textureIndex: number,
  w: number,
  h: number,
  d: number,
  cx: number,
  cy: number,
  cz: number,
  uvScale: number = 1
) {
  const geo = new THREE.BoxGeometry(w, h, d)
  const uv = geo.getAttribute('uv')
  const pos = geo.getAttribute('position')
  for (let vi = 0; vi < pos.count; vi++) {
    const px = pos.getX(vi) + cx
    const py = pos.getY(vi) + cy
    const pz = pos.getZ(vi) + cz
    const face = Math.floor(vi / 4)
    if (face <= 1) {
      uv.setXY(vi, pz, py) // ±X faces
    } else if (face <= 3) {
      uv.setXY(vi, px, pz) // ±Y faces
    } else {
      uv.setXY(vi, px, py) // ±Z faces
    }
  }
  // bakedGeo applies uvScale to every UV (its uvScaleX/Y params).
  entries.push({
    geo: bakedGeo(geo, cx, cy, cz, 0, uvScale, uvScale),
    textureIndex,
  })
}

/** Incremental quad-mesh accumulator: pass each rectangle in any corner order
 *  with its outward normal — winding is corrected against the normal, and UVs
 *  use the same axis-projection as addBox (scaled by uvScale). `finish` pushes
 *  everything accumulated as one GeoEntry. */
export function quadMeshBuilder(uvScale: number = 1) {
  const positions: number[] = []
  const normals: number[] = []
  const uvs: number[] = []
  const indices: number[] = []
  const addQuad = (
    c0: THREE.Vector3,
    c1: THREE.Vector3,
    c2: THREE.Vector3,
    c3: THREE.Vector3,
    n: THREE.Vector3
  ) => {
    const base = positions.length / 3
    const gn = c1.clone().sub(c0).cross(c2.clone().sub(c0))
    const verts = gn.dot(n) < 0 ? [c0, c3, c2, c1] : [c0, c1, c2, c3]
    const ax = Math.abs(n.x)
    const ay = Math.abs(n.y)
    const az = Math.abs(n.z)
    for (const c of verts) {
      positions.push(c.x, c.y, c.z)
      normals.push(n.x, n.y, n.z)
      let u: number, v: number
      if (ax >= ay && ax >= az) {
        u = c.z
        v = c.y
      } else if (ay >= ax && ay >= az) {
        u = c.x
        v = c.z
      } else {
        u = c.x
        v = c.y
      }
      uvs.push(u * uvScale, v * uvScale)
    }
    indices.push(base, base + 1, base + 2, base, base + 2, base + 3)
  }
  const finish = (entries: GeoEntry[], textureIndex: number) => {
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3))
    geo.setAttribute('normal', new THREE.Float32BufferAttribute(normals, 3))
    geo.setAttribute('uv', new THREE.Float32BufferAttribute(uvs, 2))
    geo.setIndex(indices)
    entries.push({ geo, textureIndex })
  }
  return { addQuad, finish }
}

const _roofMat = new THREE.Matrix4()

/**
 * Gabled (맞배지붕) roof over a rectangular footprint, centered at (cx, cz).
 * The ridge runs along the run axis (the long, `runLen` direction); the two
 * slopes face the lateral (`latW`) sides and a triangular gable closes each
 * run-axis end. Eaves overhang by `oh` on the lateral sides and by `endOh`
 * past each gable end. Pushes GeoEntry slabs for the caller to merge.
 * `omitEndSign` (−1/+1) skips that run-axis end's gable triangle — used at the
 * entry end, where the front wall supplies the gable instead.
 */
export function addGableRoof(
  entries: GeoEntry[],
  texIdx: number,
  alongZ: boolean,
  cx: number,
  cz: number,
  runLen: number,
  latW: number,
  bottomY: number,
  rise: number,
  oh: number,
  endOh: number,
  thick: number,
  omitEndSign: number = 0
) {
  const halfLat = latW / 2
  const ridgeLen = runLen + endOh * 2
  const slopeAngle = Math.atan2(rise, halfLat)
  const eaveDropY = (oh * rise) / halfLat
  const slopeLen =
    ((halfLat + oh) * Math.sqrt(halfLat * halfLat + rise * rise)) / halfLat

  // Two slope slabs. Built ridge-along-X, then rotated to Z for along-Z shafts.
  // The ridge end is mitered so the two slabs' outer faces meet flush at the
  // peak instead of leaving a gap (same technique as the house gabled roof).
  const ridgeExt = (thick * rise) / halfLat
  const totalSlopeLen = slopeLen + ridgeExt
  for (const side of [-1, 1] as const) {
    const geo = new THREE.BoxGeometry(ridgeLen, thick, totalSlopeLen)
    const uv = geo.getAttribute('uv')
    for (let i = 0; i < uv.count; i++) {
      uv.setXY(i, uv.getX(i) * ridgeLen, uv.getY(i) * totalSlopeLen)
    }
    // Pull the inner (underside) vertices at the ridge end outward by ridgeExt
    // so the slab's top edge forms the peak with no overlap or gap.
    const pos = geo.getAttribute('position')
    const innerY = -thick / 2
    const ridgeEndZ = (-side * totalSlopeLen) / 2
    for (let i = 0; i < pos.count; i++) {
      if (
        Math.abs(pos.getY(i) - innerY) < 1e-3 &&
        Math.abs(pos.getZ(i) - ridgeEndZ) < 1e-3
      ) {
        pos.setZ(i, ridgeEndZ + side * ridgeExt)
      }
    }
    geo.translate(0, thick / 2, (-side * ridgeExt) / 2)
    _roofMat.makeRotationX(side * slopeAngle)
    geo.applyMatrix4(_roofMat)
    if (alongZ) {
      _roofMat.makeRotationY(Math.PI / 2)
      geo.applyMatrix4(_roofMat)
    }
    const perpCenter = (side * (halfLat + oh)) / 2
    const yCenter = bottomY + (rise - eaveDropY) / 2
    const tx = cx + (alongZ ? perpCenter : 0)
    const tz = cz + (alongZ ? 0 : perpCenter)
    _roofMat.makeTranslation(tx, yCenter, tz)
    geo.applyMatrix4(_roofMat)
    entries.push({ geo, textureIndex: texIdx })
  }

  // Triangular gable wall at each run-axis end (base at bottomY, apex at ridge).
  for (const endSign of [-1, 1] as const) {
    if (endSign === omitEndSign) continue
    const shape = new THREE.Shape()
    shape.moveTo(-halfLat, 0)
    shape.lineTo(halfLat, 0)
    shape.lineTo(0, rise)
    shape.closePath()
    const geo = new THREE.ShapeGeometry(shape) // XY plane, normal +Z
    if (alongZ) {
      _roofMat.makeRotationY(endSign === 1 ? 0 : Math.PI)
    } else {
      _roofMat.makeRotationY(endSign === 1 ? Math.PI / 2 : -Math.PI / 2)
    }
    geo.applyMatrix4(_roofMat)
    const tx = cx + (alongZ ? 0 : (endSign * runLen) / 2)
    const tz = cz + (alongZ ? (endSign * runLen) / 2 : 0)
    _roofMat.makeTranslation(tx, bottomY, tz)
    geo.applyMatrix4(_roofMat)
    entries.push({ geo, textureIndex: texIdx })
  }
}

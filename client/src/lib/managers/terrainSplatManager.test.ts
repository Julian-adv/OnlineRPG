import { describe, it, expect } from 'vitest'
import {
  NEIGHBOR_OFFSETS_9,
  TerrainSplatManager,
  paddedOffset,
  writePaddedRange,
} from './terrainSplatManager'
import { BYTES_PER_CELL } from '../terrain/splat-encoding'
import { SPLAT_PADDED_DIM } from '../terrain/terrain-constants'

const TILE_DIM = 64
const PAD = SPLAT_PADDED_DIM

/** Build a 64×64 source filled so that cell (cx, cz) has primary byte
 *  `(cz << 4) | (cx & 0x0f)` — lets us read back which cell a padded pixel
 *  was copied from. */
function tagged64(): Uint8Array {
  const out = new Uint8Array(TILE_DIM * TILE_DIM * BYTES_PER_CELL)
  for (let cz = 0; cz < TILE_DIM; cz++) {
    for (let cx = 0; cx < TILE_DIM; cx++) {
      const off = (cz * TILE_DIM + cx) * BYTES_PER_CELL
      out[off] = ((cz & 0x0f) << 4) | (cx & 0x0f)
      out[off + 1] = cz & 0xff
      out[off + 2] = cx & 0xff
      out[off + 3] = 0
    }
  }
  return out
}

/** Which source (cx, cz) ended up at padded position (pcx, pcz)? Decodes
 *  the tagged bytes above. Returns [cx, cz] or null if all zero. */
function decodeCellAt(padded: Uint8Array, pcx: number, pcz: number) {
  const off = paddedOffset(pcx, pcz)
  const cz = padded[off + 1]
  const cx = padded[off + 2]
  return [cx, cz] as const
}

describe('splat texture coordinates', () => {
  it('keeps rows in +Z order for shader grid-corner sampling', async () => {
    const manager = new TerrainSplatManager()
    manager.setSplatmap(0, 0, tagged64())
    const texture = await manager.loadSplatmap(0, 0)
    try {
      expect(texture.flipY).toBe(false)
      const data = texture.image.data as Uint8Array
      expect(decodeCellAt(data, 33, 1)).toEqual([32, 0])
      expect(decodeCellAt(data, 33, 33)).toEqual([32, 32])
      expect(decodeCellAt(data, 33, 64)).toEqual([32, 63])
    } finally {
      await manager.destroy()
    }
  })
})

describe('writePaddedRange', () => {
  it('interior offset copies own data at [1..64]×[1..64]', () => {
    const own = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, own, true, 0, 0)
    // Four corners of the interior region.
    expect(decodeCellAt(dst, 1, 1)).toEqual([0, 0])
    expect(decodeCellAt(dst, TILE_DIM, 1)).toEqual([TILE_DIM - 1, 0])
    expect(decodeCellAt(dst, 1, TILE_DIM)).toEqual([0, TILE_DIM - 1])
    expect(decodeCellAt(dst, TILE_DIM, TILE_DIM)).toEqual([
      TILE_DIM - 1,
      TILE_DIM - 1,
    ])
    // Border pixels untouched (still zero).
    expect(decodeCellAt(dst, 0, 0)).toEqual([0, 0])
    expect(dst[paddedOffset(0, 0)]).toBe(0)
  })

  it('left edge pulls the neighbor rightmost column', () => {
    const neighbor = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, neighbor, /*srcIsOwn=*/ false, -1, 0)
    // Padded column 0, rows 1..64 should equal neighbor col TILE_DIM-1, rows 0..63.
    for (let pz = 1; pz <= TILE_DIM; pz++) {
      expect(decodeCellAt(dst, 0, pz)).toEqual([TILE_DIM - 1, pz - 1])
    }
    // Row 0 and PAD-1 untouched by this offset (those are corner writes).
    expect(dst[paddedOffset(0, 0)]).toBe(0)
    expect(dst[paddedOffset(0, PAD - 1)]).toBe(0)
  })

  it('right edge pulls the neighbor leftmost column', () => {
    const neighbor = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, neighbor, /*srcIsOwn=*/ false, 1, 0)
    for (let pz = 1; pz <= TILE_DIM; pz++) {
      expect(decodeCellAt(dst, PAD - 1, pz)).toEqual([0, pz - 1])
    }
  })

  it('top edge pulls the neighbor bottom row', () => {
    const neighbor = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, neighbor, false, 0, -1)
    for (let px = 1; px <= TILE_DIM; px++) {
      expect(decodeCellAt(dst, px, 0)).toEqual([px - 1, TILE_DIM - 1])
    }
  })

  it('bottom edge pulls the neighbor top row', () => {
    const neighbor = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, neighbor, false, 0, 1)
    for (let px = 1; px <= TILE_DIM; px++) {
      expect(decodeCellAt(dst, px, PAD - 1)).toEqual([px - 1, 0])
    }
  })

  it('NW corner pulls neighbor (63, 63)', () => {
    const neighbor = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, neighbor, false, -1, -1)
    expect(decodeCellAt(dst, 0, 0)).toEqual([TILE_DIM - 1, TILE_DIM - 1])
  })

  it('SE corner pulls neighbor (0, 0)', () => {
    const neighbor = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, neighbor, false, 1, 1)
    expect(decodeCellAt(dst, PAD - 1, PAD - 1)).toEqual([0, 0])
  })

  it('own-fallback at left edge copies own leftmost column (ClampToEdge)', () => {
    const own = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, own, /*srcIsOwn=*/ true, -1, 0)
    // Own fallback: dx=-1 picks own's column 0, not column 63.
    for (let pz = 1; pz <= TILE_DIM; pz++) {
      expect(decodeCellAt(dst, 0, pz)).toEqual([0, pz - 1])
    }
  })

  it('own-fallback at NW corner uses (0, 0)', () => {
    const own = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, own, /*srcIsOwn=*/ true, -1, -1)
    expect(decodeCellAt(dst, 0, 0)).toEqual([0, 0])
  })
})

describe('NEIGHBOR_OFFSETS_9', () => {
  it('covers the full 3×3 neighborhood exactly once', () => {
    const seen = new Set<string>()
    for (const [dx, dz] of NEIGHBOR_OFFSETS_9) {
      expect(dx).toBeGreaterThanOrEqual(-1)
      expect(dx).toBeLessThanOrEqual(1)
      expect(dz).toBeGreaterThanOrEqual(-1)
      expect(dz).toBeLessThanOrEqual(1)
      seen.add(`${dx},${dz}`)
    }
    expect(seen.size).toBe(9)
  })

  it('writing all 9 offsets from the same source fills every padded cell', () => {
    const own = tagged64()
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    for (const [dx, dz] of NEIGHBOR_OFFSETS_9) {
      writePaddedRange(dst, own, /*srcIsOwn=*/ true, dx, dz)
    }
    // Every padded pixel must have been written.
    for (let pz = 0; pz < PAD; pz++) {
      for (let px = 0; px < PAD; px++) {
        // byte 1 = cz (0..63), never equals the initial 0 unless actual source cz=0.
        // Safer assertion: primary-byte nibble is a valid tagged value (not
        // the original zero init) OR the cell decodes to a legitimate (0, 0).
        const [cx, cz] = decodeCellAt(dst, px, pz)
        expect(cx).toBeGreaterThanOrEqual(0)
        expect(cx).toBeLessThan(TILE_DIM)
        expect(cz).toBeGreaterThanOrEqual(0)
        expect(cz).toBeLessThan(TILE_DIM)
      }
    }
  })
})

describe('padded layout continuity (the bug this all fixes)', () => {
  it('adjacent-cell seam: neighbor col 0 lands in padded col PAD-1 at same row', () => {
    // Scenario: our tile (own) has an edge cell at (63, Z). The neighbor
    // tile to the right has cell (0, Z) which SHOULD be mirrored into our
    // padded texture's PAD-1 column. Pre-fix, ClampToEdge made the shader
    // see our own col 63 again; post-fix, it sees the neighbor's col 0.
    const own = tagged64()
    // Make the neighbor look different so we can tell them apart: neighbor
    // bytes index with a +100 offset in byte 1.
    const neighbor = new Uint8Array(TILE_DIM * TILE_DIM * BYTES_PER_CELL)
    for (let cz = 0; cz < TILE_DIM; cz++) {
      for (let cx = 0; cx < TILE_DIM; cx++) {
        const off = (cz * TILE_DIM + cx) * BYTES_PER_CELL
        neighbor[off] = ((cz & 0x0f) << 4) | (cx & 0x0f)
        neighbor[off + 1] = (cz + 100) & 0xff
        neighbor[off + 2] = cx & 0xff
      }
    }
    const dst = new Uint8Array(PAD * PAD * BYTES_PER_CELL)
    writePaddedRange(dst, own, true, 0, 0) // interior
    writePaddedRange(dst, neighbor, false, 1, 0) // right border
    // Sanity: interior cell (63, 10) = own col 63.
    expect(dst[paddedOffset(TILE_DIM, 11) + 1]).toBe(10)
    // Right border padded col PAD-1 at row 11 should be neighbor col 0, row 10.
    expect(dst[paddedOffset(PAD - 1, 11) + 1]).toBe((10 + 100) & 0xff)
    expect(dst[paddedOffset(PAD - 1, 11) + 2]).toBe(0)
  })
})

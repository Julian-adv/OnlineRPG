/**
 * Read animation clip durations out of a GLB, with no GLB library.
 *
 * GLB (glTF 2.0 binary) is a 12-byte header followed by chunks of
 * length(4) + type(4) + data; type 0x4E4F534A is the JSON chunk. A clip's
 * duration is the `max` of its samplers' input (time keyframe) accessors,
 * which glTF records in the accessor itself — so no buffer decoding is needed.
 *
 * Shared by tools/extract-animation-durations.mjs (player clips) and
 * tools/measure-monster-attack-clips.mjs (monster swing lengths).
 */
import { readFileSync } from 'fs'

export function parseGlbJson(filePath) {
  const buf = readFileSync(filePath)

  const magic = buf.readUInt32LE(0)
  if (magic !== 0x46546c67) {
    // 'glTF'
    throw new Error(`Not a GLB file: ${filePath}`)
  }

  const chunkLength = buf.readUInt32LE(12)
  const chunkType = buf.readUInt32LE(16)
  if (chunkType !== 0x4e4f534a) {
    // 'JSON'
    throw new Error(`First chunk is not JSON in ${filePath}`)
  }

  return JSON.parse(buf.toString('utf8', 20, 20 + chunkLength))
}

/** Clip name -> duration in seconds, rounded to millisecond precision. */
export function extractDurations(filePath) {
  const gltf = parseGlbJson(filePath)
  const result = {}
  if (!gltf.animations) return result

  for (const anim of gltf.animations) {
    const name = anim.name
    if (!name) continue

    let maxTime = 0
    for (const sampler of anim.samplers || []) {
      const accessorIdx = sampler.input
      if (accessorIdx == null || !gltf.accessors) continue
      const accessor = gltf.accessors[accessorIdx]
      if (accessor && accessor.max) {
        const t = Array.isArray(accessor.max) ? accessor.max[0] : accessor.max
        if (t > maxTime) maxTime = t
      }
    }

    result[name] = Math.round(maxTime * 1000) / 1000
  }

  return result
}

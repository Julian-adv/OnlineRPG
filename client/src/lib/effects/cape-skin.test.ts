import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import * as THREE from 'three'
import { createCapeRig } from './cape-rig'

const OPTIONS = {
  topWidth: 0.4,
  bottomWidth: 0.55,
  length: 0.9,
  segments: 5,
}

const PRINT = '/api/cape-texture/aa'

/** The loader wants a DOM image. The stub hands back a texture immediately,
 *  which also runs the node graph the print is composited with — an invalid
 *  TSL expression throws here rather than at the first frame on a GPU. */
let requested: string[]

beforeEach(() => {
  requested = []
  vi.spyOn(THREE.TextureLoader.prototype, 'load').mockImplementation(
    (url, onLoad) => {
      requested.push(url)
      const map = new THREE.Texture() as THREE.Texture<HTMLImageElement>
      onLoad?.(map)
      return map
    }
  )
})

afterEach(() => vi.restoreAllMocks())

describe('cape skins', () => {
  /** The crowd's cost: everyone in one look shares a single material and a
   *  single set of GPU buffers, not one each. */
  it('shares one material per skin and keeps prints out of each other', () => {
    const plain = createCapeRig({ ...OPTIONS, skin: { color: 0x112233 } })
    const twin = createCapeRig({ ...OPTIONS, skin: { color: 0x112233 } })
    const printed = createCapeRig({
      ...OPTIONS,
      skin: { color: 0x112233, texture: PRINT },
    })

    expect(plain.mesh.material).toBe(twin.mesh.material)
    expect(printed.mesh.material).not.toBe(plain.mesh.material)
    expect(printed.mesh.geometry).toBe(plain.mesh.geometry)
    expect(requested).toEqual([PRINT])
    expect(
      (
        printed.mesh.material as THREE.MeshStandardMaterial & {
          colorNode?: unknown
        }
      ).colorNode
    ).toBeTruthy()
    expect(
      (
        plain.mesh.material as THREE.MeshStandardMaterial & {
          colorNode?: unknown
        }
      ).colorNode
    ).toBeFalsy()

    plain.dispose()
    twin.dispose()
    printed.dispose()
  })

  /** Re-dyeing or re-printing must not rebuild the cloth: the pickers change
   *  the skin continuously, and a rebuilt skeleton drops the sheet back to its
   *  rest pose every frame. */
  it('swaps the material in place and leaves the cloth alone', () => {
    const rig = createCapeRig({ ...OPTIONS, skin: { color: 0x112233 } })
    const geometry = rig.mesh.geometry
    const skeleton = rig.mesh.skeleton
    const plain = rig.mesh.material

    rig.setSkin({ color: 0x112233, texture: PRINT })

    expect(rig.mesh.material).not.toBe(plain)
    expect(rig.mesh.geometry).toBe(geometry)
    expect(rig.mesh.skeleton).toBe(skeleton)

    const printed = rig.mesh.material
    rig.setSkin({ color: 0x112233, texture: PRINT })
    expect(rig.mesh.material).toBe(printed)
    // ...and the print behind it is not fetched a second time.
    expect(requested).toEqual([PRINT])

    rig.dispose()
  })
})

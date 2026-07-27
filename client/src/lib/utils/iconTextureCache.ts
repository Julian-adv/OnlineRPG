import * as THREE from 'three'

/**
 * Module-level cache of inventory-icon textures (a finite set of 128×128
 * PNGs), mirroring gltfCache: one decode + GPU upload per icon for the
 * process lifetime. Cached textures are shared — never dispose them.
 */
const cache = new Map<string, Promise<THREE.Texture>>()
const loader = new THREE.TextureLoader()

export function loadIconTexture(url: string): Promise<THREE.Texture> {
  let promise = cache.get(url)
  if (!promise) {
    promise = loader
      .loadAsync(url)
      .then((tex) => {
        tex.colorSpace = THREE.SRGBColorSpace
        return tex
      })
      .catch((err) => {
        cache.delete(url)
        throw err
      })
    cache.set(url, promise)
  }
  return promise
}

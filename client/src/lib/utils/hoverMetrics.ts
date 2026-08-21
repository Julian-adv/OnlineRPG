import * as THREE from 'three'

/** Ring pad beyond the model footprint, matching the ground-item hover ring. */
const RING_PAD = 0.08

/** Anchors for a model's hover name label and target ring, in its local
 *  frame. Origins often sit off-center (a stall's at one leg), so the
 *  footprint center is measured rather than assumed. */
export interface HoverMetrics {
  topY: number
  ringRadius: number
  center: { x: number; z: number }
}

export function hoverMetrics(object: THREE.Object3D): HoverMetrics {
  const box = new THREE.Box3().setFromObject(object)
  return {
    topY: box.max.y,
    ringRadius:
      Math.max(box.max.x - box.min.x, box.max.z - box.min.z) / 2 + RING_PAD,
    center: {
      x: (box.min.x + box.max.x) / 2,
      z: (box.min.z + box.max.z) / 2,
    },
  }
}

import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { bridgeManager } from './bridgeManager'
import type {
  BridgeMeta,
  ObjectDef,
  ObjectPlacement,
} from '../stores/editorStore'

const meta: BridgeMeta = {
  deckMinX: -1.75,
  deckMaxX: 1.75,
  deckMinZ: -10,
  deckMaxZ: 10,
  deckCrownY: 2,
  deckEndY: 0.3,
  deckAxis: 'z',
}

/** Arched deck: y = crown - (crown-end) * (z/10)^2, with a 1 m hole at z∈[4,5]. */
function archedDeck(): THREE.Group {
  const seg = 40
  const geo = new THREE.PlaneGeometry(3.5, 20, 1, seg)
  geo.rotateX(-Math.PI / 2)
  const pos = geo.getAttribute('position')
  for (let i = 0; i < pos.count; i++) {
    const z = pos.getZ(i)
    pos.setY(i, 2 - 1.7 * (z / 10) ** 2)
  }
  const idx = geo.getIndex()!
  const kept: number[] = []
  for (let t = 0; t < idx.count; t += 3) {
    const zs = [0, 1, 2].map((k) => pos.getZ(idx.getX(t + k)))
    const zc = (zs[0] + zs[1] + zs[2]) / 3
    if (zc > 4 && zc < 5) continue
    kept.push(idx.getX(t), idx.getX(t + 1), idx.getX(t + 2))
  }
  geo.setIndex(kept)
  const g = new THREE.Group()
  g.add(new THREE.Mesh(geo))
  return g
}

const def: ObjectDef = {
  id: 'arch',
  name: 'arch',
  kind: 'bridge',
  bridge: meta,
} as ObjectDef
const placement = {
  id: 1,
  type: 'arch',
  x: 100,
  y: 5,
  z: 200,
  rotation: 90,
  floorLevel: 0,
} as ObjectPlacement

describe('bridgeManager deck grid', () => {
  bridgeManager.registerBridgeMesh('arch', archedDeck(), meta)
  bridgeManager.syncRegion(0, 0, [placement], new Map([['arch', def]]))

  it('follows the deck curve through the placement rotation', () => {
    // rotation 90° maps local +z onto world +x
    expect(bridgeManager.findDeckYAt(100, 200, null)).toBeCloseTo(7, 1)
    expect(bridgeManager.findDeckYAt(106, 200, null)).toBeCloseTo(
      5 + 2 - 1.7 * 0.36,
      1
    )
  })

  it('rejects a point over a deck hole', () => {
    expect(bridgeManager.findDeckYAt(104.5, 200, 6)).toBeNull()
  })

  it('keeps the deck edges solid when the span is not a step multiple', () => {
    const real: BridgeMeta = {
      ...meta,
      deckMinX: -1.77,
      deckMaxX: 1.77,
      deckMinZ: -10.15,
      deckMaxZ: 10.15,
      deckCrownY: 0.35,
      deckEndY: 0.35,
    }
    const geo = new THREE.PlaneGeometry(3.54, 20.3)
    geo.rotateX(-Math.PI / 2)
    geo.translate(0, 0.35, 0)
    const g = new THREE.Group()
    g.add(new THREE.Mesh(geo))
    bridgeManager.registerBridgeMesh('flat', g, real)
    const flatDef = { ...def, id: 'flat', bridge: real } as ObjectDef
    const flat = {
      ...placement,
      id: 2,
      type: 'flat',
      x: 0,
      y: 0,
      z: 0,
      rotation: 0,
    } as ObjectPlacement
    bridgeManager.syncRegion(1, 0, [flat], new Map([['flat', flatDef]]))
    expect(bridgeManager.findDeckYAt(1.76, 0, null)).toBeCloseTo(0.35, 3)
    expect(bridgeManager.findDeckYAt(0, 10.14, null)).toBeCloseTo(0.35, 3)
    expect(bridgeManager.findDeckYAt(-1.76, -10.14, null)).toBeCloseTo(0.35, 3)
  })

  it('evicts bridges of distant regions only', () => {
    bridgeManager.evictDistant(1, 1)
    expect(bridgeManager.findDeckYAt(100, 200, null)).not.toBeNull()
    bridgeManager.evictDistant(5, 5)
    expect(bridgeManager.findDeckYAt(100, 200, null)).toBeNull()
    expect(bridgeManager.findDeckYAt(0, 0, null)).toBeNull()
  })
})

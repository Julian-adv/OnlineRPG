import { beforeEach, describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import {
  characterPanelVisible,
  inventoryVisible,
  worldMapVisible,
} from './debugStore'
import {
  closeTopOverlay,
  openOverlays,
  setOverlayCloser,
  withOverlay,
  withoutOverlay,
} from './overlayStack'

beforeEach(() => {
  characterPanelVisible.set(false)
  inventoryVisible.set(false)
  worldMapVisible.set(false)
  setOverlayCloser('worldMap', null)
})

describe('withOverlay', () => {
  it('appends a newly opened overlay', () => {
    expect(withOverlay([], 'inventory')).toEqual(['inventory'])
    expect(withOverlay(['inventory'], 'character')).toEqual([
      'inventory',
      'character',
    ])
  })

  it('moves a reopened overlay back to the top', () => {
    expect(withOverlay(['inventory', 'character'], 'inventory')).toEqual([
      'character',
      'inventory',
    ])
  })
})

describe('withoutOverlay', () => {
  it('drops the overlay wherever it sits', () => {
    expect(withoutOverlay(['inventory', 'character'], 'inventory')).toEqual([
      'character',
    ])
  })

  it('leaves the stack alone when the overlay is not open', () => {
    expect(withoutOverlay(['inventory'], 'worldMap')).toEqual(['inventory'])
  })
})

describe('openOverlays', () => {
  it('follows the order the panels were opened in', () => {
    inventoryVisible.set(true)
    characterPanelVisible.set(true)
    expect(get(openOverlays)).toEqual(['inventory', 'character'])
  })

  it('drops a panel closed by its own button', () => {
    inventoryVisible.set(true)
    characterPanelVisible.set(true)
    inventoryVisible.set(false)
    expect(get(openOverlays)).toEqual(['character'])
  })
})

describe('closeTopOverlay', () => {
  it('closes one panel per press, newest first', () => {
    inventoryVisible.set(true)
    characterPanelVisible.set(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(characterPanelVisible)).toBe(false)
    expect(get(inventoryVisible)).toBe(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(inventoryVisible)).toBe(false)
    expect(get(openOverlays)).toEqual([])
  })

  it('closes the world map ahead of a panel opened before it', () => {
    inventoryVisible.set(true)
    worldMapVisible.set(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(worldMapVisible)).toBe(false)
    expect(get(inventoryVisible)).toBe(true)
  })

  it('reports nothing to close when no overlay is open', () => {
    expect(closeTopOverlay()).toBe(false)
  })

  it('runs a registered closer instead of flipping the store itself', () => {
    let closerCalls = 0
    setOverlayCloser('worldMap', () => {
      closerCalls++
      worldMapVisible.set(false)
    })
    worldMapVisible.set(true)

    expect(closeTopOverlay()).toBe(true)
    expect(closerCalls).toBe(1)
    expect(get(worldMapVisible)).toBe(false)
  })
})

import { beforeEach, describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import {
  characterPanelVisible,
  inventoryVisible,
  settingsVisible,
  worldMapVisible,
} from './debugStore'
import { shopSession, type ShopSession } from './tradeStore'
import {
  closeTopOverlay,
  mountOverlay,
  openOverlays,
  setOverlayCloser,
  topOverlay,
  withOverlay,
  withoutOverlay,
} from './overlayStack'

const SESSION: ShopSession = {
  merchantPlayerId: 1,
  merchantName: 'Rica',
  catalog: [],
  sellRatePercent: 50,
  wishlist: [],
  stock: [],
  buyback: [],
}

beforeEach(() => {
  characterPanelVisible.set(false)
  inventoryVisible.set(false)
  worldMapVisible.set(false)
  settingsVisible.set(false)
  shopSession.set(null)
  setOverlayCloser('worldMap', null)
  setOverlayCloser('respawn', null)
  openOverlays.set([])
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

describe('topOverlay', () => {
  it('breaks a same-layer tie with the most recently opened', () => {
    expect(topOverlay(['inventory', 'character'])).toBe('character')
    expect(topOverlay(['character', 'inventory'])).toBe('inventory')
  })

  it('prefers the higher layer over the later open', () => {
    expect(topOverlay(['settings', 'inventory'])).toBe('settings')
    expect(topOverlay(['trade', 'character'])).toBe('trade')
    expect(topOverlay(['worldMap', 'inventory'])).toBe('worldMap')
  })

  it('paints the world map over the trade window despite its lower z-index', () => {
    expect(topOverlay(['worldMap', 'trade'])).toBe('worldMap')
    expect(topOverlay(['trade', 'worldMap'])).toBe('worldMap')
  })

  it('ranks the fullscreen dialogs by their paint order', () => {
    expect(topOverlay(['loading', 'respawn'])).toBe('respawn')
    expect(topOverlay(['respawn', 'worldMap'])).toBe('worldMap')
    expect(topOverlay(['respawn', 'settings'])).toBe('settings')
  })

  it('has no top when nothing is open', () => {
    expect(topOverlay([])).toBeUndefined()
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

  it('tracks the trade window through its shop session', () => {
    shopSession.set(SESSION)
    expect(get(openOverlays)).toEqual(['trade'])
    shopSession.set(null)
    expect(get(openOverlays)).toEqual([])
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

  it('closes the world map ahead of a panel it paints over', () => {
    inventoryVisible.set(true)
    worldMapVisible.set(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(worldMapVisible)).toBe(false)
    expect(get(inventoryVisible)).toBe(true)
  })

  it('still closes the world map first when the panel was opened last', () => {
    worldMapVisible.set(true)
    inventoryVisible.set(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(worldMapVisible)).toBe(false)
    expect(get(inventoryVisible)).toBe(true)
  })

  it('closes settings before an inventory opened underneath it', () => {
    inventoryVisible.set(true)
    settingsVisible.set(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(settingsVisible)).toBe(false)
    expect(get(inventoryVisible)).toBe(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(inventoryVisible)).toBe(false)
  })

  it('closes settings first even when the panel was opened over it', () => {
    settingsVisible.set(true)
    inventoryVisible.set(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(settingsVisible)).toBe(false)
    expect(get(inventoryVisible)).toBe(true)
  })

  it('closes the trade window before a character sheet under it', () => {
    characterPanelVisible.set(true)
    shopSession.set(SESSION)

    expect(closeTopOverlay()).toBe(true)
    expect(get(shopSession)).toBeNull()
    expect(get(characterPanelVisible)).toBe(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(characterPanelVisible)).toBe(false)
  })

  it('defers the respawn dialog before touching a panel behind it', () => {
    inventoryVisible.set(true)
    let laterCalls = 0
    const unmount = mountOverlay('respawn', () => laterCalls++)

    expect(closeTopOverlay()).toBe(true)
    expect(laterCalls).toBe(1)
    expect(get(inventoryVisible)).toBe(true)

    unmount()
    expect(closeTopOverlay()).toBe(true)
    expect(get(inventoryVisible)).toBe(false)
  })

  it('never closes a panel hidden behind the loading dialog', () => {
    characterPanelVisible.set(true)
    const unmount = mountOverlay('loading')

    expect(closeTopOverlay()).toBe(false)
    expect(get(characterPanelVisible)).toBe(true)

    unmount()
    expect(closeTopOverlay()).toBe(true)
    expect(get(characterPanelVisible)).toBe(false)
  })

  it('still closes the world map painted above the loading dialog', () => {
    const unmount = mountOverlay('loading')
    worldMapVisible.set(true)

    expect(closeTopOverlay()).toBe(true)
    expect(get(worldMapVisible)).toBe(false)
    expect(closeTopOverlay()).toBe(false)

    unmount()
  })

  it('reports nothing to close when no overlay is open', () => {
    expect(closeTopOverlay()).toBe(false)
  })

  it('runs a registered closer instead of resetting the state itself', () => {
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

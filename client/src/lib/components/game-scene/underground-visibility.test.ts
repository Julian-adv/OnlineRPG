import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

/**
 * The overworld is hidden in dungeons by one `$effect` in GameScene.svelte.
 * A layer left out of it keeps rendering underground — that is how the shop
 * sign stayed visible from inside a dungeon. These tests fail when a layer
 * exposing getGroup() is neither gated nor explicitly exempted.
 */

const GAME_SCENE = join(__dirname, '..', 'GameScene.svelte')
const LAYER_DIRS = [__dirname, join(__dirname, '..', 'map-editor')]

/**
 * Layers whose group must be hidden while underground, mapped to the variable
 * the effect assigns `.visible` on. Asserting the assignment, not just that the
 * group is reached — `objectGroup.visible = true` must not pass.
 */
const SURFACE_LAYERS: Record<string, string> = {
  GameSceneTerrainLayer: 'terrainGroup',
  GameSceneWaterFieldLayer: 'waterGroup',
  GameSceneGrassLayer: 'grassGroup',
  GameSceneTreeLayer: 'treeGroup',
  GameSceneHousingLayer: 'housingGroup',
  GameSceneWindParticles: 'windGroup',
  GameSceneRiverRocksLayer: 'riverRocksGroup',
  GameSceneShoreSprayLayer: 'shoreSprayGroup',
  ObjectOverlay: 'objectGroup',
}

/** Layers deliberately outside the gate, with the reason they are exempt. */
const EXEMPT_LAYERS: Record<string, string> = {
  GameSceneDungeonLayer: 'renders the dungeon itself',
  GameSceneGroundItemsLayer: 'filters by dungeon depth on its own',
}

function componentsExposingGetGroup(): string[] {
  const found: string[] = []
  for (const dir of LAYER_DIRS) {
    for (const file of readdirSync(dir)) {
      if (!file.endsWith('.svelte')) continue
      const src = readFileSync(join(dir, file), 'utf8')
      if (src.includes('export function getGroup')) {
        found.push(file.replace(/\.svelte$/, ''))
      }
    }
  }
  return found
}

/** The body of the `$effect` that hides the overworld underground. */
function undergroundEffectBody(src: string): string {
  const start = src.indexOf('// Underground render mode')
  expect(start, 'underground render-mode effect not found').toBeGreaterThan(-1)
  const end = src.indexOf('\n  })', start)
  expect(end, 'underground effect has no closing brace').toBeGreaterThan(start)
  return src.slice(start, end)
}

describe('underground overworld gate', () => {
  const src = readFileSync(GAME_SCENE, 'utf8')

  it('hides every surface layer while underground', () => {
    const body = undergroundEffectBody(src)
    for (const [component, groupVar] of Object.entries(SURFACE_LAYERS)) {
      expect(
        new RegExp(`\\b${groupVar}\\.visible = !underground\\b`).test(body),
        `${component} is not hidden underground`
      ).toBe(true)
    }
  })

  it('leaves no visibility assignment in the effect ungated', () => {
    const body = undergroundEffectBody(src)
    const assignments = body.match(/\w+\.visible = [^\n]+/g) ?? []
    expect(assignments.length).toBe(Object.keys(SURFACE_LAYERS).length)
    for (const assignment of assignments) {
      expect(assignment, 'ungated visibility assignment').toContain(
        '= !underground'
      )
    }
  })

  it('drops the object overlay from pick targets underground', () => {
    // THREE's raycaster ignores .visible, so hiding the group is not enough:
    // surface signs stay hoverable and clickable unless removed from the set.
    const pickSet = src.match(/pickableObjectMeshes = \$derived\([^)]*\)/s)
    expect(pickSet, 'pickableObjectMeshes not found').not.toBeNull()
    expect(pickSet![0]).toContain('!$isUnderground')
    expect(src).toContain('objectMeshes={pickableObjectMeshes}')
  })

  it('classifies every layer that exposes getGroup', () => {
    const classified = new Set([
      ...Object.keys(SURFACE_LAYERS),
      ...Object.keys(EXEMPT_LAYERS),
    ])
    const unclassified = componentsExposingGetGroup().filter(
      (c) => !classified.has(c)
    )
    expect(
      unclassified,
      'new layer: add it to SURFACE_LAYERS or EXEMPT_LAYERS'
    ).toEqual([])
  })
})

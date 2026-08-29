import type * as THREE from 'three'
import type { WebGPURenderer } from 'three/webgpu'
import type Monster from '../Monster.svelte'
import type PlayerModel from '../PlayerModel.svelte'
import type GameSceneWaterFieldLayer from './GameSceneWaterFieldLayer.svelte'
import type GameSceneRiverRocksLayer from './GameSceneRiverRocksLayer.svelte'
import type GameSceneShoreSprayLayer from './GameSceneShoreSprayLayer.svelte'
import type GameSceneGrassLayer from './GameSceneGrassLayer.svelte'
import type GameSceneTreeLayer from './GameSceneTreeLayer.svelte'
import type GameSceneWindParticles from './GameSceneWindParticles.svelte'
import type GameSceneHousingLayer from './GameSceneHousingLayer.svelte'
import type ObjectOverlay from '../map-editor/ObjectOverlay.svelte'
import type { RefractionRenderManager } from '../../managers/refractionRenderManager'
import type { ReflectionRenderManager } from '../../managers/reflectionRenderManager'
import type { LoopProfiler } from './loop-profiler'
import type { RenderProfiler } from './render-profiler'
import type { MultiPassRenderer } from './multi-pass-rendering'

export interface RenderPassesContext {
  renderer: WebGPURenderer
  camera: THREE.OrthographicCamera | undefined
  multiPassRenderer: MultiPassRenderer
  refractionManager: RefractionRenderManager | null
  reflectionManager: ReflectionRenderManager | null
  refractionEnabled: boolean
  reflectionEnabled: boolean
  waterGroup: THREE.Group | undefined
  hasWater: boolean
  terrainGroup: THREE.Group | undefined
  terrainMeshes: (THREE.Mesh | undefined)[]
  entityClipGroup: THREE.Group | undefined
  waterLayerRef: GameSceneWaterFieldLayer | undefined
  riverRocksRef: GameSceneRiverRocksLayer | undefined
  shoreSprayRef: GameSceneShoreSprayLayer | undefined
  grassLayerRef: GameSceneGrassLayer | undefined
  treeLayerRef: GameSceneTreeLayer | undefined
  windParticlesRef: GameSceneWindParticles | undefined
  housingLayerRef: GameSceneHousingLayer | undefined
  objectOverlayRef: ObjectOverlay | undefined
  currentPlayerModel: PlayerModel | null
  otherPlayerModels: (PlayerModel | undefined)[]
  monsterModels: (Monster | undefined)[]
  renderProfiler: RenderProfiler
  loopProfiler: LoopProfiler
}

export function runRenderPasses(ctx: RenderPassesContext): void {
  // Wetness pre-pass is not gated behind multiPassReady — it's a tiny
  // 256x256 RT per water tile with negligible pipeline overhead, and
  // deferring it causes blocky wet sand on the first visible frame.
  const wetnessStart = performance.now()
  ctx.renderProfiler.withTag('wetness', () => {
    ctx.waterLayerRef?.renderWetness(ctx.renderer)
  })
  ctx.loopProfiler.record('wetnessPass', performance.now() - wetnessStart)

  ctx.multiPassRenderer.tickWarmup()

  ctx.renderProfiler.withTag('refraction', () => {
    ctx.multiPassRenderer.renderRefraction(
      {
        camera: ctx.camera,
        refractionManager: ctx.refractionManager,
        refractionEnabled: ctx.refractionEnabled,
        hasWater: ctx.hasWater,
        waterGroup: ctx.waterGroup,
        terrainMeshes: ctx.terrainMeshes,
        hiddenGroups: aboveWaterGroups(ctx),
      },
      ctx.loopProfiler
    )
  })

  ctx.renderProfiler.withTag('reflection', () => {
    ctx.multiPassRenderer.renderReflection(
      {
        camera: ctx.camera,
        reflectionManager: ctx.reflectionManager,
        reflectionEnabled: ctx.reflectionEnabled,
        hasWater: ctx.hasWater,
        waterGroup: ctx.waterGroup,
        terrainGroup: ctx.terrainGroup,
        housingGroup: ctx.housingLayerRef?.getGroup(),
        hiddenGroups: aboveWaterGroups(ctx),
        getNametagGroups: () => collectNametagGroups(ctx),
      },
      ctx.loopProfiler
    )
  })
}

/** Everything above the water surface: neither refracted into the bed image
 *  nor mirrored, and every group left in costs a pipeline per material. */
function aboveWaterGroups(
  ctx: RenderPassesContext
): (THREE.Group | undefined)[] {
  return [
    ctx.entityClipGroup,
    ctx.grassLayerRef?.getGroup(),
    ctx.treeLayerRef?.getGroup(),
    ctx.windParticlesRef?.getGroup(),
    ctx.objectOverlayRef?.getGroup(),
    ctx.riverRocksRef?.getGroup(),
    ctx.shoreSprayRef?.getGroup(),
  ]
}

function collectNametagGroups(ctx: RenderPassesContext): THREE.Group[] {
  const groups: THREE.Group[] = []
  const ntCurrent = ctx.currentPlayerModel?.getNametagGroup()
  if (ntCurrent) groups.push(ntCurrent)
  for (const pm of ctx.otherPlayerModels) {
    const nt = pm?.getNametagGroup()
    if (nt) groups.push(nt)
  }
  for (const mm of ctx.monsterModels) {
    const nt = mm?.getNametagGroup()
    if (nt) groups.push(nt)
  }
  return groups
}

export function recordRenderProfilerStats(
  renderProfiler: RenderProfiler,
  loopProfiler: LoopProfiler
): void {
  loopProfiler.record('mainRenderCpu', renderProfiler.ms.main)
  loopProfiler.recordCount('mainRenderCalls', renderProfiler.renderCalls.main)
  loopProfiler.recordCount('mainDraws', renderProfiler.drawCalls.main)
  loopProfiler.recordCount('mainTrisK', renderProfiler.triangles.main / 1000)
  loopProfiler.recordCount(
    'refractionDraws',
    renderProfiler.drawCalls.refraction
  )
  loopProfiler.recordCount(
    'refractionTrisK',
    renderProfiler.triangles.refraction / 1000
  )
  loopProfiler.recordCount(
    'reflectionDraws',
    renderProfiler.drawCalls.reflection
  )
  loopProfiler.recordCount(
    'reflectionTrisK',
    renderProfiler.triangles.reflection / 1000
  )
  loopProfiler.recordCount('wetnessDraws', renderProfiler.drawCalls.wetness)
}

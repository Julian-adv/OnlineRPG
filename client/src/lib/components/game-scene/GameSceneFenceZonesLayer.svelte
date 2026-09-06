<script lang="ts">
  import { useTask } from '@threlte/core'
  import type { LocalPlayer } from '../../stores/gameStore'
  import { editorZoneManager } from '../../stores/editorStore'
  import type { NoSpawnZone, ZoneData } from '../../managers/zoneManager'
  import { worldToTileCoord } from '../../managers/terrain-height-types'
  import {
    REGION_CELLS,
    TILE_DIM,
    tileToRegion,
  } from '../../terrain/terrain-constants'
  import {
    wrapRegionX,
    WORLD_MIN_REGION_Z,
    WORLD_MAX_REGION_Z,
  } from '../../terrain/world-wrap'
  import ZoneOverlay from '../map-editor/ZoneOverlay.svelte'

  let { player }: { player: LocalPlayer } = $props()
  let tileX = $state<number | null>(null)
  let tileZ = $state<number | null>(null)
  let data = $state<ZoneData>({ noSpawnZones: [] })

  useTask(() => {
    tileX = worldToTileCoord(player.position.x)
    tileZ = worldToTileCoord(player.position.z)
  })

  $effect(() => {
    const manager = $editorZoneManager
    if (!manager || tileX === null || tileZ === null) return
    const x = tileX * TILE_DIM
    const z = tileZ * TILE_DIM
    const radius = TILE_DIM * 2
    const region = (value: number) => tileToRegion(worldToTileCoord(value))
    const requests: Promise<NoSpawnZone[]>[] = []
    let cancelled = false
    data = { noSpawnZones: [] }
    for (let rx = region(x - radius); rx <= region(x + radius); rx++) {
      for (
        let rz = Math.max(WORLD_MIN_REGION_Z, region(z - radius));
        rz <= Math.min(WORLD_MAX_REGION_Z, region(z + radius));
        rz++
      ) {
        const wrappedX = wrapRegionX(rx)
        const offsetX = (rx - wrappedX) * REGION_CELLS
        requests.push(
          manager.fetchZone(wrappedX, rz).then((zones) =>
            (zones.noSpawnZones ?? [])
              .map((zone) => ({
                ...zone,
                minX: zone.minX + offsetX,
                maxX: zone.maxX + offsetX,
              }))
              .filter(
                (zone) =>
                  zone.maxX >= x - radius &&
                  zone.minX <= x + radius &&
                  zone.maxZ >= z - radius &&
                  zone.minZ <= z + radius
              )
          )
        )
      }
    }
    Promise.all(requests).then((regions) => {
      if (!cancelled) data = { noSpawnZones: regions.flat() }
    })
    return () => {
      cancelled = true
    }
  })
</script>

<ZoneOverlay {data} readOnly />

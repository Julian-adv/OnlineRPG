<script lang="ts">
  import { T } from '@threlte/core'
  import * as THREE from 'three'
  import { onDestroy } from 'svelte'
  import { activeDebuffs } from '../../stores/debuffStore'
  import { debuffDurationMs } from '../../data/debuffPresentation'
  import { WetFootprints, STRIDE_M } from '../../effects/wet-footprints'
  import {
    shortestWrappedDeltaX,
    unwrapWorldXNear,
  } from '../../terrain/world-wrap'
  import type { PlayerState } from '../../utils/movementUtils'
  import type { RemotePlayer } from '../../stores/gameStore'

  /**
   * Wet footprints behind anyone carrying the `wet` soaking (doc/DEBUFF.md).
   *
   * The local player's own trail comes from `activeDebuffs` (owner-only, so
   * it knows the remaining time and fades the prints as they dry). Remote
   * trails ride the broadcast `Player.wet` flag, which carries no countdown —
   * their prints keep one strength — and only `high` asks for them.
   */

  interface Props {
    playerPosition?: { x: number; y: number; z: number } | null
    /** Interpolated remote-player poses, the same source their models use. */
    remotePlayers?: Map<number, PlayerState>
    otherPlayers?: Map<number, RemotePlayer>
    enableRemote?: boolean
    /** Baked water surface height at a world XZ (sea level where none). */
    waterSurfaceAt?: (x: number, z: number) => number
  }

  let {
    playerPosition = null,
    remotePlayers = undefined,
    otherPlayers = undefined,
    enableRemote = false,
    waterSurfaceAt = undefined,
  }: Props = $props()

  const WET_DURATION_MS = debuffDurationMs('wet')
  /** A frame moving farther than this is a teleport, not a stride. */
  const TELEPORT_M = 5
  /** Remote prints get no countdown, so they take a fixed strength. */
  const REMOTE_STRENGTH = 0.7
  /** Feet this far under the water surface leave nothing to see. */
  const SUBMERGED_M = 0.02

  const system = new WetFootprints()

  export function getGroup(): THREE.Group {
    return system.group
  }

  let wetUntil = 0
  const unsubscribe = activeDebuffs.subscribe((debuffs) => {
    wetUntil = debuffs.find((d) => d.id === 'wet')?.until ?? 0
  })
  onDestroy(() => {
    unsubscribe()
    system.dispose()
  })

  interface Stride {
    x: number
    z: number
    walked: number
    side: number
  }
  let localStride: Stride | null = null
  /* eslint-disable-next-line svelte/prefer-svelte-reactivity */
  const remoteStrides = new Map<number, Stride>()

  /** Accumulate one walker's travel and stamp a print each full stride. */
  function trail(
    last: Stride | null,
    x: number,
    y: number,
    z: number,
    strength: number
  ): Stride {
    if (!last) return { x, z, walked: 0, side: 1 }
    const dx = shortestWrappedDeltaX(last.x, x)
    const dz = z - last.z
    const moved = Math.hypot(dx, dz)
    last.x = x
    last.z = z
    // Teleports and respawns land here; the seam wrap does not, since the
    // delta above takes the short way round.
    if (moved > TELEPORT_M) {
      last.walked = 0
      return last
    }
    last.walked += moved
    if (last.walked < STRIDE_M) return last
    last.walked = 0
    // Wading leaves no visible trail — the print would sit on the riverbed,
    // under the water surface.
    if ((waterSurfaceAt?.(x, z) ?? -Infinity) - y > SUBMERGED_M) return last
    system.emit(x, y, z, dx, dz, last.side, strength)
    last.side = -last.side
    return last
  }

  /** Called from GameScene's game loop each frame (deltaTime in ms). */
  export function update(deltaTime: number) {
    system.update(deltaTime / 1000)

    // Checked at emit time, so the trail stops the instant the soaking
    // expires even between the server's sweeps.
    const remaining = wetUntil - Date.now()
    if (playerPosition && remaining > 0) {
      // The last of the water leaves fainter prints.
      const strength = 0.45 + 0.45 * Math.min(remaining / WET_DURATION_MS, 1)
      localStride = trail(
        localStride,
        playerPosition.x,
        playerPosition.y,
        playerPosition.z,
        strength
      )
    } else {
      localStride = null
    }

    if (!enableRemote || !remotePlayers || !otherPlayers || !playerPosition) {
      remoteStrides.clear()
      return
    }
    for (const [id, pose] of remotePlayers) {
      if (!otherPlayers.get(id)?.wet) {
        remoteStrides.delete(id)
        continue
      }
      // Their model is drawn unwrapped near the viewer; the prints have to
      // land under it, not a world width away.
      const x = unwrapWorldXNear(playerPosition.x, pose.position.x)
      remoteStrides.set(
        id,
        trail(
          remoteStrides.get(id) ?? null,
          x,
          pose.position.y,
          pose.position.z,
          REMOTE_STRENGTH
        )
      )
    }
    // Anyone who walked out of range keeps no accumulator.
    for (const id of remoteStrides.keys()) {
      if (!remotePlayers.has(id)) remoteStrides.delete(id)
    }
  }
</script>

<T is={system.group} />

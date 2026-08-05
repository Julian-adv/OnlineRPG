<script lang="ts">
  import { T } from '@threlte/core'
  import { onDestroy } from 'svelte'
  import * as THREE from 'three'
  import { MeshBasicNodeMaterial } from 'three/webgpu'
  import type { Position } from '../../utils/movementUtils'
  import type { TerrainHeightManager } from '../../managers/terrainHeightManager'
  import {
    TELEPORT_GATES,
    TELEPORT_GATE_MISFIRE_CHANCE_BPS,
  } from '../../data/teleportGateDefs'
  import { currentDungeonDepth } from '../../stores/dungeonStore'

  interface Props {
    heightManager: TerrainHeightManager
    playerPosition: Position | null
  }

  let { heightManager, playerPosition }: Props = $props()

  const VISIBLE_RANGE_SQ = 220 * 220
  const group = new THREE.Group()
  group.name = 'town-teleport-gates'

  const stoneMaterial = new THREE.MeshStandardMaterial({
    color: 0x526173,
    roughness: 0.82,
    metalness: 0.08,
  })
  const trimMaterial = new THREE.MeshStandardMaterial({
    color: 0x9ec8df,
    roughness: 0.34,
    metalness: 0.58,
    emissive: 0x123954,
    emissiveIntensity: 0.75,
  })
  const signMaterial = new THREE.MeshStandardMaterial({
    color: 0x241b16,
    roughness: 0.88,
  })
  const portalMaterial = new MeshBasicNodeMaterial()
  portalMaterial.color = new THREE.Color(0x42bfff)
  portalMaterial.transparent = true
  portalMaterial.opacity = 0.28
  portalMaterial.depthWrite = false
  portalMaterial.side = THREE.DoubleSide
  const runeMaterial = new MeshBasicNodeMaterial()
  runeMaterial.color = new THREE.Color(0xa8e7ff)
  runeMaterial.transparent = true
  runeMaterial.opacity = 0.9

  const sharedGeometries = [
    new THREE.TorusGeometry(2.35, 0.34, 16, 64),
    new THREE.TorusGeometry(1.94, 0.055, 8, 64),
    new THREE.CircleGeometry(1.9, 64),
    new THREE.CylinderGeometry(2.8, 3.15, 0.35, 24),
    new THREE.BoxGeometry(3.65, 1.18, 0.16),
    new THREE.BoxGeometry(0.75, 0.58, 0.72),
  ]
  const [
    ringGeometry,
    runeGeometry,
    portalGeometry,
    baseGeometry,
    signGeometry,
    keystoneGeometry,
  ] = sharedGeometries

  interface GateVisual {
    root: THREE.Group
    portal: THREE.Mesh
    runes: THREE.Mesh
    textTexture: THREE.CanvasTexture
    textMaterial: MeshBasicNodeMaterial
    textGeometry: THREE.PlaneGeometry
  }

  // Imperative scene objects are updated by the main game loop.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const visuals = new Map<string, GateVisual>()

  function buildNoticeTexture(townName: string): THREE.CanvasTexture {
    const canvas = document.createElement('canvas')
    canvas.width = 1024
    canvas.height = 360
    const ctx = canvas.getContext('2d')!
    ctx.clearRect(0, 0, canvas.width, canvas.height)
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.strokeStyle = '#1a0f08'
    ctx.lineJoin = 'round'

    const line = (text: string, y: number, size: number, color: string) => {
      ctx.font = `bold ${size}px Georgia, serif`
      ctx.lineWidth = Math.max(5, size * 0.11)
      ctx.strokeText(text, canvas.width / 2, y)
      ctx.fillStyle = color
      ctx.fillText(text, canvas.width / 2, y)
    }
    line(`${townName.toUpperCase()} TOWN GATE`, 76, 54, '#e8d8ae')
    line('FARES RISE WITH DISTANCE', 176, 37, '#bce6ff')
    line(
      `${(TELEPORT_GATE_MISFIRE_CHANCE_BPS / 100).toFixed(2)}% MISFIRE: LAND/SEA/DUNGEON`,
      270,
      37,
      '#ffd37b'
    )

    const texture = new THREE.CanvasTexture(canvas)
    texture.colorSpace = THREE.SRGBColorSpace
    texture.minFilter = THREE.LinearFilter
    texture.magFilter = THREE.LinearFilter
    return texture
  }

  for (const gate of TELEPORT_GATES) {
    const root = new THREE.Group()
    root.name = `town-gate-${gate.id}`
    root.position.set(gate.x, 0, gate.z)
    root.rotation.y = gate.rotation
    root.userData.teleportGateId = gate.id

    const base = new THREE.Mesh(baseGeometry, stoneMaterial)
    base.position.y = 0.12
    base.receiveShadow = true
    root.add(base)

    const ring = new THREE.Mesh(ringGeometry, stoneMaterial)
    ring.position.y = 2.7
    ring.castShadow = true
    ring.receiveShadow = true
    root.add(ring)

    const portal = new THREE.Mesh(portalGeometry, portalMaterial)
    portal.position.set(0, 2.7, 0.035)
    root.add(portal)

    const runes = new THREE.Mesh(runeGeometry, runeMaterial)
    runes.position.set(0, 2.7, 0.1)
    root.add(runes)

    const keystone = new THREE.Mesh(keystoneGeometry, trimMaterial)
    keystone.position.set(0, 5.05, 0)
    keystone.rotation.z = Math.PI / 4
    keystone.castShadow = true
    root.add(keystone)

    const sign = new THREE.Mesh(signGeometry, signMaterial)
    sign.position.set(0, 1.05, 3.05)
    sign.castShadow = true
    root.add(sign)

    const textTexture = buildNoticeTexture(gate.name)
    const textMaterial = new MeshBasicNodeMaterial()
    textMaterial.map = textTexture
    textMaterial.transparent = true
    textMaterial.depthWrite = false
    textMaterial.side = THREE.DoubleSide
    const textGeometry = new THREE.PlaneGeometry(3.48, 1.02)
    const text = new THREE.Mesh(textGeometry, textMaterial)
    text.position.set(0, 1.05, 3.145)
    root.add(text)

    const glow = new THREE.PointLight(0x55c7ff, 2.2, 11, 2)
    glow.position.set(0, 2.7, 0.6)
    root.add(glow)

    root.visible = false
    group.add(root)
    visuals.set(gate.id, {
      root,
      portal,
      runes,
      textTexture,
      textMaterial,
      textGeometry,
    })
  }

  export function update(deltaTimeMs: number) {
    const underground = $currentDungeonDepth >= 1
    const time = performance.now() / 1000
    for (const gate of TELEPORT_GATES) {
      const visual = visuals.get(gate.id)!
      const dx = (playerPosition?.x ?? Infinity) - gate.x
      const dz = (playerPosition?.z ?? Infinity) - gate.z
      visual.root.visible =
        !underground && dx * dx + dz * dz <= VISIBLE_RANGE_SQ
      if (!visual.root.visible) continue

      visual.root.position.y = heightManager.getHeightAtWorldPosition(
        gate.x,
        gate.z
      )
      visual.runes.rotation.z += deltaTimeMs * 0.00035
      const pulse = 1 + Math.sin(time * 2.4) * 0.025
      visual.portal.scale.setScalar(pulse)
    }
  }

  export function getGroup(): THREE.Group {
    return group
  }

  onDestroy(() => {
    for (const visual of visuals.values()) {
      // CanvasTexture-backed node materials must be left to GC. Explicitly
      // disposing them can poison WebGPU's shared sampler bindings; see the
      // same lifecycle rule in TextLabel.svelte and shop-sign.ts.
      visual.textGeometry.dispose()
    }
    for (const geometry of sharedGeometries) geometry.dispose()
    stoneMaterial.dispose()
    trimMaterial.dispose()
    signMaterial.dispose()
    portalMaterial.dispose()
    runeMaterial.dispose()
  })
</script>

<T is={group} />

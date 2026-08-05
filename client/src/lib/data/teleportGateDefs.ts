import teleportGatesJson from '../../../../data/teleport_gates.json'
import teleportGateConfigJson from '../../../../data/teleport_gate_config.json'

export interface TeleportGateDefinition {
  id: string
  name: string
  x: number
  z: number
  rotation: number
}

interface TeleportGateConfig {
  id: string
  interactionRangeM: number
  arrivalOffsetM: number
  baseFareCopper: number
  farePerKmCopper: number
  misfireChanceBps: number
  dungeonMisfirePercent: number
}

export const TELEPORT_GATES = Object.values(
  teleportGatesJson as Record<string, TeleportGateDefinition>
).sort((a, b) => a.name.localeCompare(b.name))

export const TELEPORT_GATE_CONFIG = (
  teleportGateConfigJson as Record<string, TeleportGateConfig>
).town_network
export const TELEPORT_GATE_INTERACTION_RANGE_METERS =
  TELEPORT_GATE_CONFIG.interactionRangeM
export const TELEPORT_GATE_MISFIRE_CHANCE_BPS =
  TELEPORT_GATE_CONFIG.misfireChanceBps
export const TELEPORT_GATE_WARNING = `Fares rise with distance. There is a ${(
  TELEPORT_GATE_MISFIRE_CHANCE_BPS / 100
).toFixed(2)}% chance of a wild gate misfire to land, sea, or a dungeon.`

export function getTeleportGate(
  id: string
): TeleportGateDefinition | undefined {
  return TELEPORT_GATES.find((gate) => gate.id === id)
}

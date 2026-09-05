import merchantsJson from '../../../../data/merchants.json'

export interface MerchantDefinition {
  id: string
  npcName: string
  sellRatePercent: number
  /** Semicolon-separated item def ids. */
  catalog: string
}

const merchantDefs = merchantsJson as Record<string, MerchantDefinition>

const byNpcName = new Map(
  Object.values(merchantDefs).map((def) => [def.npcName, def])
)

/** Merchant lookup by NPC character name (NPCs are agent-controlled players).
 *  Non-merchant traders live in traderDefs.ts; use getNpcCapabilities there
 *  to decide how an NPC can be interacted with. */
export function getMerchantByNpcName(
  npcName: string
): MerchantDefinition | undefined {
  return byNpcName.get(npcName)
}

const stocked = new Set(
  Object.values(merchantDefs).flatMap((def) =>
    def.catalog.split(';').map((id) => id.trim())
  )
)

/** Whether any merchant shelf carries the item. */
export function isStockedByAnyMerchant(itemDefId: string): boolean {
  return stocked.has(itemDefId)
}

export default merchantDefs

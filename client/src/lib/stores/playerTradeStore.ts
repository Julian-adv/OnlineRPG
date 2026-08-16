import { writable } from 'svelte/store'
import { enqueueConsent } from './consentQueue'

/** One item on the table (ServerMessage::PlayerTradeUpdate). `enchant` is
 *  displayed in the name, never only in a tooltip: +0 and +7 are otherwise
 *  identical, which is the cheapest scam in the system. */
export interface PlayerTradeItem {
  instance_id: number
  item_def_id: string
  quantity: number
  enchant: number
}

export interface PlayerTradeSide {
  player_id: number
  name: string
  items: PlayerTradeItem[]
  copper: number
  locked: boolean
  confirmed: boolean
}

/** The live session. `you` is always this client's own side. */
export interface PlayerTradeState {
  revision: number
  you: PlayerTradeSide
  them: PlayerTradeSide
}

/** Null when no trade is open; the window mounts off this. */
export const playerTrade = writable<PlayerTradeState | null>(null)

/** Last rejection from the server (stale revision, overweight, untradeable),
 *  shown inside the window. Cleared on the next update. */
export const playerTradeError = writable<string | null>(null)

export interface PendingTradeRequest {
  requesterId: number
  requesterName: string
  offeredAt: number
}

export const pendingTradeRequests = writable<PendingTradeRequest[]>([])

/** Mirrors the server's PLAYER_TRADE_REQUEST_TTL. */
export const TRADE_REQUEST_TTL_MS = 30_000

/** Matches the party/friend pending caps. */
const TRADE_REQUEST_CAP = 5

export function enqueueTradeRequest(
  requesterId: number,
  requesterName: string
) {
  enqueueConsent(
    pendingTradeRequests,
    TRADE_REQUEST_CAP,
    (entry) => entry.requesterId === requesterId,
    { requesterId, requesterName, offeredAt: Date.now() }
  )
}

export function dismissTradeRequest(requesterId: number) {
  pendingTradeRequests.update((queue) =>
    queue.filter((entry) => entry.requesterId !== requesterId)
  )
}

/** Units of `instanceId` this player has on the table. The bag panel greys
 *  out this many rather than the whole slot, because partial stacks are
 *  offerable and re-laying out the bag mid-trade invites misclicks. */
export function reservedQuantity(
  state: PlayerTradeState | null,
  instanceId: number
): number {
  if (!state) return 0
  return state.you.items
    .filter((item) => item.instance_id === instanceId)
    .reduce((sum, item) => sum + item.quantity, 0)
}

export function resetPlayerTrade() {
  playerTrade.set(null)
  playerTradeError.set(null)
  pendingTradeRequests.set([])
}

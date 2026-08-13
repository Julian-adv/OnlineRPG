/** Server-validated maximum player↔merchant distance for any shop
 *  interaction. Must match MAX_TRADE_DISTANCE in
 *  server/src/game_state/trading.rs. */
export const MAX_TRADE_DISTANCE_METERS = 6

/** Client-side range for opening a shop by clicking a merchant; kept below
 *  the server limit so the player never lands in an error state. */
export const NPC_TRADE_RANGE_METERS = 5

/** Client reach for tipping a clicked tip hat; kept under the server's 5m so
 *  a step taken while the dialog is open never voids the tip. */
export const TIP_HAT_RANGE_METERS = 3

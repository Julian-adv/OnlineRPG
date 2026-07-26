/** Fixed simulation step. Every in-game scene mutation runs on this cadence,
 *  so rendering faster only re-presents identical state. */
export const TARGET_FPS = 60

export const FRAME_TIME_MS = 1000 / TARGET_FPS

/** Slack for the simulation gate. Tight because a frame rejected here is
 *  recovered by the next one's stepCount catch-up — nothing is lost. */
export const FRAME_TOLERANCE_MS = 0.5

/** Slack for the render gate, as a fraction of the render interval (1ms at
 *  60fps). Wider and proportional because a dropped render never comes back. */
export const RENDER_TOLERANCE_RATIO = 0.06

import { persistedBoolean } from './persisted'

/** HUD minimap toggle. Off by default (issue #11); persisted per browser. */
export const minimapEnabled = persistedBoolean(
  'onlinerpg_minimapEnabled',
  false
)

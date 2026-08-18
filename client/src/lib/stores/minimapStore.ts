import { persistedBoolean } from './persisted'

/** HUD minimap toggle, persisted per browser. */
export const minimapEnabled = persistedBoolean('onlinerpg_minimapEnabled', true)

import { writable } from 'svelte/store'

/** Open dye dialog, holding the bottle that will be spent — set when the
 *  server answers a `UseItem` with `CapeDyePrompt`. */
export const capeDyeDialog = writable<{ instanceId: number } | null>(null)

/** Colour the picker is trying on. Overrides the worn cape's own colour for
 *  the local player while the dialog is open; cancelling clears it. */
export const capeDyePreview = writable<string | null>(null)

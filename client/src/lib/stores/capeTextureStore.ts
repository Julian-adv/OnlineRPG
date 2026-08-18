import { writable } from 'svelte/store'

/** Open transfer-kit dialog, holding the kit that will be spent — set when
 *  the server answers a `UseItem` with `CapeTexturePrompt`. */
export const capeTextureDialog = writable<{ instanceId: number } | null>(null)

/** Object URL of the picture the dialog is trying on. Overrides the worn
 *  cape's print for the local player while the dialog is open; cancelling
 *  clears it. */
export const capeTexturePreview = writable<string | null>(null)

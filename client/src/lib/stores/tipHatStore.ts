import { writable } from 'svelte/store'

/** Open tip dialog target, or null when the dialog is closed. */
export const tipHatDialog = writable<{
  hatId: number
  ownerName: string
} | null>(null)

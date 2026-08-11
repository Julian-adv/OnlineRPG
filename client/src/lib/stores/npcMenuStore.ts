import { writable } from 'svelte/store'

/** One entry in the NPC right-click context menu. Actions are closures
 *  built by PlayerControl (which owns movement/network), so the menu
 *  component stays purely presentational. */
export interface NpcMenuEntry {
  label: string
  action: () => void
}

/** An open NPC context menu, anchored at screen coordinates. Null = closed. */
export interface NpcContextMenuState {
  npcName: string
  screenX: number
  screenY: number
  entries: NpcMenuEntry[]
}

export const npcContextMenu = writable<NpcContextMenuState | null>(null)

export function closeNpcContextMenu() {
  npcContextMenu.set(null)
}

/** Incremented to ask the chat panel to focus its input (the "Talk"
 *  default action for non-merchant NPCs). */
export const chatFocusRequest = writable(0)

export function requestChatFocus() {
  chatFocusRequest.update((n) => n + 1)
}

/** Ask the chat panel to put `text` in its input and focus it (the friend
 *  panel's Whisper button). `seq` rather than the text alone, so whispering
 *  the same friend twice in a row still applies. */
export const chatDraftRequest = writable({ text: '', seq: 0 })

export function requestChatDraft(text: string) {
  chatDraftRequest.update((request) => ({ text, seq: request.seq + 1 }))
}

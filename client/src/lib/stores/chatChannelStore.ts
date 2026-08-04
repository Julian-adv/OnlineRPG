import { writable } from 'svelte/store'

/** Where plain input lines go: local say or the party channel. Sticky — a
 *  `/p` (or the input-bar toggle) holds until `/s` switches back; leaving
 *  the party reverts it (see ChatPanel's roster effect). */
export type ChatChannel = 'say' | 'party'

export const chatChannel = writable<ChatChannel>('say')

/** Losing the party reverts the input to say — but only once the draft is
 *  empty. A pending line keeps the party channel, so it can only fall into
 *  the server's private not-in-a-party refusal, never into public chat. */
export function shouldRevertToSay(
  inParty: boolean,
  channel: ChatChannel,
  draft: string
): boolean {
  return !inParty && channel === 'party' && draft.trim().length === 0
}

export function shouldBlockNpcTalkForPartyDraft(
  channel: ChatChannel,
  draft: string
): boolean {
  return channel === 'party' && draft.trim().length > 0
}

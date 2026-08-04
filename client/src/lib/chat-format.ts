import type { ChatEntry } from './stores/gameStore'

/** The sender gets the same whisper echoed back; direction decides the label. */
export function whisperChatEntry(
  from: string,
  to: string,
  message: string,
  ownName: string | undefined
): ChatEntry {
  const outgoing = from === ownName
  return {
    text: message,
    sender: 'whisper',
    name: outgoing ? `To ${to}` : `From ${from}`,
  }
}

/** Party lines carry the sender's name as-is; the panel adds the [Party] tag. */
export function partyChatEntry(from: string, message: string): ChatEntry {
  return { text: message, sender: 'party', name: from }
}

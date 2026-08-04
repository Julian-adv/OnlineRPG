import type { ChatEntry, StoredChatEntry } from './stores/gameStore'

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

/** Party lines from other members newer than `seenId`. Id-keyed rather than a
 *  running count: the transcript is a bounded ring, so a count would shrink as
 *  it evicts, quietly clearing messages nobody read. */
export function unreadPartyCount(
  entries: Pick<StoredChatEntry, 'sender' | 'name' | 'id'>[],
  seenId: number,
  ownName: string | undefined
): number {
  return entries.filter(
    (e) => e.sender === 'party' && e.id > seenId && e.name !== ownName
  ).length
}

/** What the Party tab shows: the channel itself plus the party and summon
 *  notices, which reach the client as plain system lines. */
export function isPartyTabLine(
  entry: Pick<ChatEntry, 'sender' | 'text'>
): boolean {
  return (
    entry.sender === 'party' ||
    (entry.sender === 'system' &&
      (entry.text.startsWith('Party:') || entry.text.startsWith('Summon:')))
  )
}

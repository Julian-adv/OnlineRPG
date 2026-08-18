export const CHAT_HISTORY_MAX = 100
const STORAGE_KEY = 'onlinerpg_chatHistory'

/** Sent lines, oldest first, browsed shell-style with the arrow keys. The
 *  draft typed before browsing comes back once the cursor walks past the
 *  newest entry; editing a recalled line ends the browse. */
export class ChatHistory {
  private entries: string[]
  private cursor: number
  private draft = ''
  private shown: string | null = null

  constructor(entries: string[] = []) {
    this.entries = entries.slice(-CHAT_HISTORY_MAX)
    this.cursor = this.entries.length
  }

  static load(): ChatHistory {
    try {
      const raw = localStorage.getItem(STORAGE_KEY)
      const parsed: unknown = raw ? JSON.parse(raw) : []
      if (Array.isArray(parsed)) {
        return new ChatHistory(parsed.filter((e) => typeof e === 'string'))
      }
    } catch {
      // unavailable or corrupt storage; start empty
    }
    return new ChatHistory()
  }

  get lines(): readonly string[] {
    return this.entries
  }

  push(line: string): void {
    if (line && this.entries.at(-1) !== line) {
      this.entries.push(line)
      if (this.entries.length > CHAT_HISTORY_MAX) this.entries.shift()
    }
    this.reset()
    this.save()
  }

  /** ArrowUp: older line, or null at the oldest. `current` is what the input
   *  holds now; if it isn't the line last handed out, browsing restarts. */
  prev(current: string): string | null {
    if (!this.browsing(current)) this.reset()
    if (this.cursor === 0) return null
    if (this.cursor === this.entries.length) this.draft = current
    this.cursor--
    return (this.shown = this.entries[this.cursor])
  }

  /** ArrowDown: newer line, the saved draft past the newest, or null when
   *  not browsing. */
  next(current: string): string | null {
    if (!this.browsing(current) || this.cursor >= this.entries.length) {
      return null
    }
    this.cursor++
    return (this.shown =
      this.cursor === this.entries.length
        ? this.draft
        : this.entries[this.cursor])
  }

  private browsing(current: string): boolean {
    return this.shown !== null && current === this.shown
  }

  private reset(): void {
    this.cursor = this.entries.length
    this.draft = ''
    this.shown = null
  }

  private save(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(this.entries))
    } catch {
      // unavailable storage; the history just won't persist
    }
  }
}

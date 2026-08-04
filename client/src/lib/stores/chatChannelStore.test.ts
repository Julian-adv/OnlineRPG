import { describe, it, expect } from 'vitest'
import { shouldRevertToSay } from './chatChannelStore'

describe('shouldRevertToSay', () => {
  it('reverts an empty draft when the party is gone', () => {
    expect(shouldRevertToSay(false, 'party', '')).toBe(true)
    expect(shouldRevertToSay(false, 'party', '   ')).toBe(true)
  })

  it('keeps a pending party draft off the public channel', () => {
    // The draft holds the party channel, so sending it draws the server's
    // private not-in-a-party refusal instead of local chat.
    expect(shouldRevertToSay(false, 'party', 'meet at the west gate')).toBe(
      false
    )
  })

  it('never reverts while the party is alive or already on say', () => {
    expect(shouldRevertToSay(true, 'party', '')).toBe(false)
    expect(shouldRevertToSay(false, 'say', '')).toBe(false)
  })
})

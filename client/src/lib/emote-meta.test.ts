import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { EMOTE_LIST } from './emote-meta'
import { LOOPING_EMOTE_ANIMS, ONE_SHOT_EMOTE_ANIMS } from './stores/emoteStore'

// The wire contract lives in shared/src/messages.rs; emoteStore mirrors it by
// hand. Parse the Rust source so drift fails here instead of shipping a panel
// that silently omits a new emote (the panel itself derives from emoteStore).
function rustEmoteList(constName: string): string[] {
  const source = readFileSync(
    path.resolve(__dirname, '../../../shared/src/messages.rs'),
    'utf-8'
  )
  // Anchored on the declaration so a doc-comment example can't hijack it.
  const match = source.match(
    new RegExp(`pub const ${constName}\\s*:[^=]*=\\s*&\\[([^\\]]*)\\]`)
  )
  if (!match) throw new Error(`${constName} not found in messages.rs`)
  return [...match[1].matchAll(/"([^"]+)"/g)].map((m) => m[1])
}

describe('emote panel metadata', () => {
  it('mirrors ONE_SHOT_EMOTES from shared/src/messages.rs', () => {
    expect(new Set(rustEmoteList('ONE_SHOT_EMOTES'))).toEqual(
      ONE_SHOT_EMOTE_ANIMS
    )
  })

  it('mirrors LOOPING_EMOTES from shared/src/messages.rs', () => {
    expect(new Set(rustEmoteList('LOOPING_EMOTES'))).toEqual(
      LOOPING_EMOTE_ANIMS
    )
  })

  it('derives one labelled entry per accepted emote', () => {
    expect(new Set(EMOTE_LIST.map((e) => e.anim))).toEqual(
      new Set([...ONE_SHOT_EMOTE_ANIMS, ...LOOPING_EMOTE_ANIMS])
    )
    for (const emote of EMOTE_LIST) {
      expect(emote.label.length).toBeGreaterThan(0)
      expect(emote.loops).toBe(LOOPING_EMOTE_ANIMS.has(emote.anim))
    }
  })
})

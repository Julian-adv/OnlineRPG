import { describe, expect, it } from 'vitest'
import { catchMessage } from './fishingMessages'

// The combat-log wording contract, category by category. Mirrors the
// agent-client's caught_line tests (agent-client/src/driver/prompt.rs).
describe('catchMessage', () => {
  it('announces a fish with its size', () => {
    expect(
      catchMessage(
        { name: 'Raw Trout', category: 'fish' },
        'raw_trout',
        34,
        false
      )
    ).toBe('You caught a Raw Trout (34 cm).')
  })

  it('celebrates a trophy regardless of category wording', () => {
    expect(
      catchMessage(
        { name: 'Golden Sturgeon', category: 'fish' },
        'golden_sturgeon',
        120,
        true
      )
    ).toBe('Trophy catch! Golden Sturgeon, 120 cm!')
  })

  it('fishes up junk without a size (a boot is not a specimen)', () => {
    expect(
      catchMessage(
        { name: 'Old Boot', category: 'junk' },
        'old_boot',
        40,
        false
      )
    ).toBe('You fished up an Old Boot.')
  })

  it('hauls up a coin catch without putting it in the bag wording', () => {
    expect(
      catchMessage(
        { name: 'Sunken Coin Pouch', category: 'coin_catch' },
        'sunken_coin_pouch',
        12,
        false
      )
    ).toBe('You haul up a Sunken Coin Pouch!')
  })

  it('picks the article by the leading vowel', () => {
    expect(
      catchMessage(
        { name: 'Iron Kettle', category: 'junk' },
        'iron_kettle',
        20,
        false
      )
    ).toBe('You fished up an Iron Kettle.')
    expect(
      catchMessage({ name: 'Kelp', category: 'junk' }, 'kelp', 20, false)
    ).toBe('You fished up a Kelp.')
  })

  it('falls back to the raw id when the def is unknown', () => {
    expect(catchMessage(undefined, 'mystery_item', 10, false)).toBe(
      'You fished up a mystery_item.'
    )
  })
})

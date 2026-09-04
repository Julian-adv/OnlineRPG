import { describe, expect, it } from 'vitest'
import { isTorchItemDefId, wornAmmoDefId } from './inventoryStore'

describe('isTorchItemDefId', () => {
  it('recognizes every carried torch variant', () => {
    expect(isTorchItemDefId('torch')).toBe(true)
    expect(isTorchItemDefId('worn_torch')).toBe(true)
  })

  it('rejects missing and unrelated item definitions', () => {
    expect(isTorchItemDefId(undefined)).toBe(false)
    expect(isTorchItemDefId(null)).toBe(false)
    expect(isTorchItemDefId('dagger')).toBe(false)
  })
})

describe('wornAmmoDefId', () => {
  const bag = (...ids: string[]) =>
    ids.map((item_def_id, i) => ({
      instance_id: i + 1,
      item_def_id,
      quantity: 10,
      enchant: 0,
    }))

  it('names the stack the hand cell is drawing', () => {
    expect(
      wornAmmoDefId({
        bag: bag('iron_arrow'),
        equipped: { main_hand: bag('bow')[0] },
        active_ammo: 'iron_arrow',
      })
    ).toBe('iron_arrow')
  })

  /** The cell only appears with a ranged weapon in hand. Hiding the stack
   *  from the bag on any other rule would put the quiver in neither place. */
  it('names nothing without a ranged weapon in hand', () => {
    expect(
      wornAmmoDefId({
        bag: bag('iron_arrow'),
        equipped: { main_hand: bag('iron_sword')[0] },
        active_ammo: 'iron_arrow',
      })
    ).toBeUndefined()
    expect(
      wornAmmoDefId({
        bag: bag('iron_arrow'),
        equipped: {},
        active_ammo: 'iron_arrow',
      })
    ).toBeUndefined()
  })

  /** The choice outlives an empty quiver, so it can name a stack that is no
   *  longer carried — nothing to hide then. */
  it('names nothing once the chosen stack is spent', () => {
    expect(
      wornAmmoDefId({
        bag: bag('steel_arrow'),
        equipped: { main_hand: bag('bow')[0] },
        active_ammo: 'iron_arrow',
      })
    ).toBeUndefined()
  })

  it('names nothing when no round is chosen', () => {
    expect(
      wornAmmoDefId({
        bag: bag('iron_arrow'),
        equipped: { main_hand: bag('bow')[0] },
        active_ammo: null,
      })
    ).toBeUndefined()
  })
})

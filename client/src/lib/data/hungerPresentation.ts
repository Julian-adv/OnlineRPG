import type { HungerBand, HungerSnapshot } from '../stores/hungerStore'

export const HUNGER_BAND_INFO = {
  WellFed: {
    label: 'Well Fed',
    icon: '🍖',
    description: 'You feel energized and ready for adventure.',
  },
  Hungry: {
    label: 'Hungry',
    icon: '🍽️',
    description: 'A meal would help, but you can keep going.',
  },
  Weak: {
    label: 'Weak',
    icon: '🦴',
    description: 'You need food before your strength returns.',
  },
  Stuffed: {
    label: 'Stuffed',
    icon: '🫃',
    description: 'The vigor returns once your meal settles.',
  },
} satisfies Record<
  HungerBand,
  { label: string; icon: string; description: string }
>

export function hungerModifiers(hunger: HungerSnapshot) {
  return [
    { label: 'Movement', mult: hunger.moveMult },
    { label: 'Attack', mult: hunger.attackMult },
    { label: 'Carry', mult: hunger.carryMult },
  ]
}

export function formatHungerModifier(multiplier: number) {
  return `${multiplier > 1 ? '+' : ''}${Math.round((multiplier - 1) * 100)}%`
}

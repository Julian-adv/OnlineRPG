import type { HungerBand, HungerSnapshot } from '../stores/hungerStore'

export const HUNGER_BAND_INFO = {
  Normal: {
    label: 'Normal',
    icon: '🍞',
    description: 'You have enough fuel to sprint and recover normally.',
    note: undefined,
  },
  Hungry: {
    label: 'Hungry',
    icon: '🍽️',
    description: 'Sprinting is unavailable and natural healing is slower.',
    note: 'Sprinting is disabled. Natural healing takes twice as long.',
  },
  Weak: {
    label: 'Weak',
    icon: '🦴',
    description: 'You need food before your strength returns.',
    note: 'Natural healing is disabled.',
  },
} satisfies Record<
  HungerBand,
  { label: string; icon: string; description: string; note: string | undefined }
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

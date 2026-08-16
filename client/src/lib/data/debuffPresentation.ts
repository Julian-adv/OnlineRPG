import debuffsJson from '../../../../data/debuffs.json'

export interface DebuffPresentation {
  label: string
  icon: string
  note: string
  applied: string
  expired: string
}

export interface PresentedDebuff extends DebuffPresentation {
  id: string
  /** Compact remaining time, e.g. "8s" or "5m". */
  remaining: string
}

const NAMES: Record<string, { name?: string }> = debuffsJson

const PRESENTATION: Record<string, Partial<DebuffPresentation>> = {
  food_poisoning: {
    icon: '☠️',
    note: 'Heavy penalties',
    applied: 'Your stomach churns — food poisoning! Cooked food next time.',
    expired: 'The sickness passes. You feel yourself again.',
  },
  bleed: {
    icon: '🩸',
    note: 'Losing HP · no natural healing',
    applied: 'You are bleeding!',
    expired: 'The bleeding stops.',
  },
}

export function debuffPresentation(id: string): DebuffPresentation {
  const label = NAMES[id]?.name ?? id
  return {
    label,
    icon: '⚠️',
    note: '',
    applied: `You are afflicted: ${label}.`,
    expired: `${label} wears off.`,
    ...PRESENTATION[id],
  }
}

export function formatRemaining(ms: number) {
  const seconds = Math.max(0, Math.ceil(ms / 1_000))
  return seconds >= 60 ? `${Math.ceil(seconds / 60)}m` : `${seconds}s`
}

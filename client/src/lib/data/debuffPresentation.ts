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

const DEFS: Record<
  string,
  { name?: string; durationSecs?: number; armorWeightMult?: number }
> = debuffsJson

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
  wet: {
    icon: '💧',
    note: 'Slowed · armour weighs more',
    applied: 'You are soaked through — heavy going until you dry off.',
    expired: 'Your clothes are dry again.',
  },
}

export function debuffPresentation(id: string): DebuffPresentation {
  const label = DEFS[id]?.name ?? id
  return {
    label,
    icon: '⚠️',
    note: '',
    applied: `You are afflicted: ${label}.`,
    expired: `${label} wears off.`,
    ...PRESENTATION[id],
  }
}

/** A debuff's full duration in ms, for effects that fade with what's left. */
export function debuffDurationMs(id: string) {
  return (DEFS[id]?.durationSecs ?? 0) * 1_000
}

/** Combined `armor` weight factor of the debuffs currently up (doc/DEBUFF.md). */
export function armorWeightMult(ids: string[]) {
  return ids.reduce((mult, id) => mult * (DEFS[id]?.armorWeightMult ?? 1), 1)
}

export function formatRemaining(ms: number) {
  const seconds = Math.max(0, Math.ceil(ms / 1_000))
  return seconds >= 60 ? `${Math.ceil(seconds / 60)}m` : `${seconds}s`
}

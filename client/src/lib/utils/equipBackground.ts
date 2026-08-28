import type { Gender } from '../network/networkTypes'

const CONCEPT_DIR = '/character_concepts'
const FINAL_FALLBACK = `${CONCEPT_DIR}/female_priest.webp`

// Female files that don't follow the female_{class}.webp convention.
const FEMALE_IRREGULAR: Record<string, string> = {
  caveman: `${CONCEPT_DIR}/cavewoman.webp`,
  valkyrie: `${CONCEPT_DIR}/valkyrie.webp`,
}

function conceptPath(characterClass: string, gender: Gender): string {
  if (gender === 'female') {
    const irregular = FEMALE_IRREGULAR[characterClass]
    if (irregular) return irregular
    return `${CONCEPT_DIR}/female_${characterClass}.webp`
  }
  return `${CONCEPT_DIR}/${characterClass}.webp`
}

const CSS_FILTER: Record<string, string> = {
  [conceptPath('barbarian', 'male')]: 'brightness(0.7)',
}

export function equipBgFilter(path: string | undefined): string | undefined {
  return path ? CSS_FILTER[path] : undefined
}

// Own gender first, then the opposite gender, then the default; the component
// advances past missing files on load error.
export function equipBgCandidates(
  characterClass: string,
  gender: Gender
): string[] {
  const opposite: Gender = gender === 'female' ? 'male' : 'female'
  const candidates = [
    conceptPath(characterClass, gender),
    conceptPath(characterClass, opposite),
    FINAL_FALLBACK,
  ]
  return candidates.filter((path, i) => candidates.indexOf(path) === i)
}

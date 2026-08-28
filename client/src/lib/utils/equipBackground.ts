import type { Gender } from '../network/networkTypes'

const CONCEPT_DIR = '/character_concepts'
const FINAL_FALLBACK = `${CONCEPT_DIR}/female_priest.png`

// Files that don't follow the {class}.png / female_{class}.png convention.
const IRREGULAR: Record<Gender, Record<string, string>> = {
  female: {
    caveman: `${CONCEPT_DIR}/cavewoman.png`,
    valkyrie: `${CONCEPT_DIR}/valkyrie.jpg`,
  },
  male: {
    valkyrie: `${CONCEPT_DIR}/valkyrie.jpg`,
  },
}

function conceptPath(characterClass: string, gender: Gender): string {
  const irregular = IRREGULAR[gender][characterClass]
  if (irregular) return irregular
  const prefix = gender === 'female' ? 'female_' : ''
  return `${CONCEPT_DIR}/${prefix}${characterClass}.png`
}

// Ordered image candidates for the equip-panel background: own gender first,
// then the opposite gender's art, then the historical default. The component
// advances to the next entry on load error, so dropping a conventionally
// named file into character_concepts/ takes effect without code changes.
// characterClass is a plain string because agent-created characters can carry
// classes outside the web client's union (same reality as classIconPath).
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

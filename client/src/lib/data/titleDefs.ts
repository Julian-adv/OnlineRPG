import { derived, get } from 'svelte/store'
import titlesJson from '../../../../data/titles.json'
import { persistedString } from '../stores/persisted'

interface TitleDef {
  id: string
  name: string
  nameKo: string
  order: number
}

const defs = titlesJson as Record<string, TitleDef>

export type TitleLanguage = 'auto' | 'ko' | 'en'
export const TITLE_LANGUAGES: { value: TitleLanguage; label: string }[] = [
  { value: 'auto', label: 'Auto' },
  { value: 'ko', label: '한국어' },
  { value: 'en', label: 'English' },
]

/** Which language titles render in; 'auto' follows the browser. */
export const titleLanguage = persistedString<TitleLanguage>(
  'onlinerpg_titleLanguage',
  'auto',
  (v): v is TitleLanguage => v === 'auto' || v === 'ko' || v === 'en'
)

const browserKorean = (navigator.language ?? '').toLowerCase().startsWith('ko')

function nameIn(id: string, korean: boolean): string {
  const def = defs[id]
  if (!def) return id
  return korean && def.nameKo ? def.nameKo : def.name
}

/** Reactive lookup: `$titleName(id)` re-renders when the setting changes. */
export const titleName = derived(
  titleLanguage,
  (lang) => (id: string) =>
    nameIn(id, lang === 'auto' ? browserKorean : lang === 'ko')
)

/** Non-reactive lookup for one-off text (chat lines). */
export function titleNameNow(id: string): string {
  return get(titleName)(id)
}

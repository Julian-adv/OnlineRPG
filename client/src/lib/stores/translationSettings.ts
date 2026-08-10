import { writable } from 'svelte/store'
import { persistedBoolean } from './persisted'

export interface TranslationLanguage {
  code: string
  label: string
}

// Chrome's Translator API accepts BCP-47 tags; this list is just the
// languages exposed in the settings UI, not an API restriction.
export const TRANSLATION_LANGUAGES: TranslationLanguage[] = [
  { code: 'en', label: 'English' },
  { code: 'ko', label: 'Korean' },
  { code: 'ja', label: 'Japanese' },
  { code: 'zh', label: 'Chinese (Simplified)' },
  { code: 'zh-Hant', label: 'Chinese (Traditional)' },
  { code: 'es', label: 'Spanish' },
  { code: 'fr', label: 'French' },
  { code: 'de', label: 'German' },
  { code: 'pt', label: 'Portuguese' },
  { code: 'ru', label: 'Russian' },
  { code: 'it', label: 'Italian' },
  { code: 'vi', label: 'Vietnamese' },
  { code: 'th', label: 'Thai' },
  { code: 'id', label: 'Indonesian' },
  { code: 'hi', label: 'Hindi' },
  { code: 'ar', label: 'Arabic' },
  { code: 'tr', label: 'Turkish' },
  { code: 'pl', label: 'Polish' },
  { code: 'nl', label: 'Dutch' },
]

const STORAGE_KEY_ENABLED = 'onlinerpg_translationEnabled'
const STORAGE_KEY_TARGET = 'onlinerpg_translationTargetLanguage'
const DEFAULT_TARGET = 'en'

export const translationEnabled = persistedBoolean(STORAGE_KEY_ENABLED, false)
export const translationTargetLanguage = writable<string>(
  localStorage.getItem(STORAGE_KEY_TARGET) ?? DEFAULT_TARGET
)

translationTargetLanguage.subscribe((v) => {
  localStorage.setItem(STORAGE_KEY_TARGET, v)
})

// Minimal typings for Chrome's built-in Translator API (not yet in lib.dom.d.ts).
type TranslatorAvailability =
  | 'unavailable'
  | 'downloadable'
  | 'downloading'
  | 'available'

interface TranslatorLanguagePair {
  sourceLanguage: string
  targetLanguage: string
}

interface TranslatorInstance {
  translate(text: string): Promise<string>
  destroy(): void
}

interface TranslatorApi {
  availability(pair: TranslatorLanguagePair): Promise<TranslatorAvailability>
  create(pair: TranslatorLanguagePair): Promise<TranslatorInstance>
}

declare const Translator: TranslatorApi | undefined

interface LanguageDetectionResult {
  detectedLanguage: string
  confidence: number
}

interface LanguageDetectorInstance {
  detect(text: string): Promise<LanguageDetectionResult[]>
  destroy(): void
}

interface LanguageDetectorApi {
  availability(): Promise<TranslatorAvailability>
  create(): Promise<LanguageDetectorInstance>
}

declare const LanguageDetector: LanguageDetectorApi | undefined

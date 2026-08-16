// Wraps Chrome's built-in Translator + LanguageDetector APIs. Source language
// is auto-detected per message (chat has many speakers, not one fixed
// source); instances are cached so repeat language pairs/detection don't each
// pay model-load cost.

export function isTranslatorApiSupported(): boolean {
  return (
    typeof Translator !== 'undefined' && typeof LanguageDetector !== 'undefined'
  )
}

const translators = new Map<string, Promise<TranslatorInstance>>()
// Pairs the API reported unavailable — memoized so new messages in such a
// pair don't re-probe availability(); transient failures stay retryable.
const unavailablePairs = new Set<string>()
let detector: Promise<LanguageDetectorInstance> | null = null

async function getTranslator(
  sourceLanguage: string,
  targetLanguage: string
): Promise<TranslatorInstance> {
  const key = `${sourceLanguage}:${targetLanguage}`
  if (unavailablePairs.has(key)) {
    throw new Error(`Translator unavailable for ${key}`)
  }
  const cached = translators.get(key)
  if (cached) return cached

  const created = (async () => {
    const availability = await Translator!.availability({
      sourceLanguage,
      targetLanguage,
    })
    if (availability === 'unavailable') {
      unavailablePairs.add(key)
      throw new Error(`Translator unavailable for ${key}`)
    }
    // 'downloadable'/'downloading' resolve once create()'s model download finishes.
    return Translator!.create({ sourceLanguage, targetLanguage })
  })()
  translators.set(key, created)
  created.catch(() => translators.delete(key))
  return created
}

async function getDetector(): Promise<LanguageDetectorInstance> {
  if (detector) return detector
  detector = (async () => {
    const availability = await LanguageDetector!.availability()
    if (availability === 'unavailable') {
      throw new Error('LanguageDetector unavailable')
    }
    return LanguageDetector!.create()
  })()
  detector.catch(() => (detector = null))
  return detector
}

/** Returns null when detection is inconclusive ('und') — caller should skip translation. */
async function detectLanguage(text: string): Promise<string | null> {
  const [top] = await (await getDetector()).detect(text)
  return top && top.detectedLanguage !== 'und' ? top.detectedLanguage : null
}

/** Never rejects — resolves with the original text when translation is
 * skipped (inconclusive detection, same language) or fails. */
export async function translateChatText(
  text: string,
  targetLanguage: string
): Promise<string> {
  try {
    const sourceLanguage = await detectLanguage(text)
    if (!sourceLanguage || sourceLanguage === targetLanguage) return text
    const translator = await getTranslator(sourceLanguage, targetLanguage)
    return await translator.translate(text)
  } catch {
    return text
  }
}

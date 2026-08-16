// Chat translation via the agent-client's configured LLM endpoint, proxied
// across the iframe boundary.

import { TRANSLATION_LANGUAGES } from '../stores/translationSettings'

const TIMEOUT_MS = 15000

let nextId = 0
const pending = new Map<number, (text: string | null) => void>()
let listening = false

function listen() {
  if (listening) return
  listening = true
  window.addEventListener('message', (event: MessageEvent) => {
    if (event.data?.type !== 'openmmo-translate-result') return
    const resolve = pending.get(event.data.id)
    if (!resolve) return
    pending.delete(event.data.id)
    resolve(typeof event.data.text === 'string' ? event.data.text : null)
  })
}

export function isLlmTranslationAvailable(): boolean {
  return typeof window !== 'undefined' && window.parent !== window
}

function labelFor(code: string): string {
  return TRANSLATION_LANGUAGES.find((l) => l.code === code)?.label ?? code
}

/** Null means "fall back to the on-device translator". */
export function translateViaHost(
  text: string,
  targetLanguage: string
): Promise<string | null> {
  if (!isLlmTranslationAvailable()) return Promise.resolve(null)
  listen()

  const id = nextId++
  return new Promise<string | null>((resolve) => {
    const done = (value: string | null) => {
      clearTimeout(timer)
      pending.delete(id)
      resolve(value)
    }
    const timer = setTimeout(() => done(null), TIMEOUT_MS)
    pending.set(id, done)
    window.parent.postMessage(
      {
        type: 'openmmo-translate',
        id,
        text,
        target: labelFor(targetLanguage),
      },
      '*'
    )
  })
}

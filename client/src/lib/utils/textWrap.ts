export type MeasureFn = (text: string) => number

const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' })

/** Break `word` into max-width chunks between graphemes (never inside a ZWJ
 *  emoji cluster); pushes full chunks, returns the tail. */
function breakChars(
  word: string,
  maxWidthPx: number,
  measure: MeasureFn,
  out: string[]
): string {
  let cur = ''
  for (const { segment } of graphemes.segment(word)) {
    const test = cur + segment
    if (measure(test) > maxWidthPx && cur.length > 0) {
      out.push(cur)
      cur = segment
    } else {
      cur = test
    }
  }
  return cur
}

function wrapWords(
  para: string,
  maxWidthPx: number,
  measure: MeasureFn,
  out: string[]
) {
  let cur = ''
  for (const w of para.split(/\s+/)) {
    if (!w) continue
    const test = cur ? cur + ' ' + w : w
    const testW = measure(test)
    if (testW <= maxWidthPx) {
      cur = test
      continue
    }
    if (cur) out.push(cur)
    // Words wider than the line break mid-word, so unspaced runs still wrap.
    cur =
      (cur ? measure(w) : testW) > maxWidthPx
        ? breakChars(w, maxWidthPx, measure, out)
        : w
  }
  if (cur) out.push(cur)
}

export function wrapLines(
  text: string,
  maxWidthPx: number,
  measure: MeasureFn
): string[] {
  const lines: string[] = []
  for (const para of text.split('\n')) {
    if (!para) {
      lines.push('')
      continue
    }
    wrapWords(para, maxWidthPx, measure, lines)
  }
  return lines.length ? lines : ['']
}

export function truncateGraphemes(text: string, max: number): string {
  let count = 0
  for (const { index } of graphemes.segment(text)) {
    if (count++ === max) return text.slice(0, index)
  }
  return text
}

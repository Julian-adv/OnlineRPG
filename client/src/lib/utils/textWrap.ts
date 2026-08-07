export type MeasureFn = (text: string) => number

const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' })

/** Break `word` into max-width chunks; pushes full chunks, returns the tail.
 *  Splits between grapheme clusters, never inside one — a mid-cluster break
 *  would turn ZWJ emoji like 👨‍👩‍👧‍👦 into different visible emoji. */
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
    if (measure(test) <= maxWidthPx) {
      cur = test
      continue
    }
    if (cur) {
      out.push(cur)
      cur = ''
    }
    // A word wider than the line on its own gets broken mid-word,
    // so unspaced runs (e.g. Korean without spaces) still wrap.
    cur = measure(w) > maxWidthPx ? breakChars(w, maxWidthPx, measure, out) : w
  }
  if (cur) out.push(cur)
}

export function wrapLines(
  text: string,
  maxWidthPx: number,
  breakWord: boolean,
  measure: MeasureFn
): string[] {
  const lines: string[] = []
  for (const para of text.split('\n')) {
    if (!para) {
      lines.push('')
      continue
    }
    if (breakWord) {
      const tail = breakChars(para, maxWidthPx, measure, lines)
      if (tail) lines.push(tail)
    } else {
      wrapWords(para, maxWidthPx, measure, lines)
    }
  }
  return lines.length ? lines : ['']
}

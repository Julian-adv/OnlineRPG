import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { compile } from 'svelte/compiler'

// Text selection is opt-in: <main> disables it app-wide and specific
// copyable surfaces re-enable it. Pure CSS, so guard the compiled output.
function compiledCss(relPath: string): string {
  const file = path.resolve(__dirname, relPath)
  const source = readFileSync(file, 'utf-8')
  return compile(source, { filename: file }).css?.code ?? ''
}

function rule(selector: string, declaration: string): RegExp {
  return new RegExp(`${selector}[^{]*\\{[^}]*${declaration}`)
}

describe('app-wide text selection policy', () => {
  it('disables selection on the app root', () => {
    const css = compiledCss('./App.svelte')
    expect(css).toMatch(rule('main\\.svelte-\\S+', 'user-select: none'))
  })

  it('re-enables selection for text entry in any child component', () => {
    const css = compiledCss('./App.svelte')
    expect(css).toMatch(rule('main\\.svelte-\\S+ input', 'user-select: text'))
    expect(css).toMatch(
      rule('main\\.svelte-\\S+ textarea', 'user-select: text')
    )
  })

  it('keeps the chat transcript copyable', () => {
    const css = compiledCss('./lib/components/ChatPanel.svelte')
    expect(css).toMatch(rule('\\.chat-messages', 'user-select: text'))
  })

  it('keeps announcement bodies copyable', () => {
    const css = compiledCss('./lib/components/AnnouncementsPanel.svelte')
    expect(css).toMatch(rule('\\.body', 'user-select: text'))
  })

  it('keeps login error messages copyable', () => {
    const css = compiledCss('./lib/components/LoginScreen.svelte')
    expect(css).toMatch(rule('\\.kicked-message', 'user-select: text'))
    expect(css).toMatch(rule('\\.error-message', 'user-select: text'))
  })

  it('keeps the body-mounted item tooltip unselectable', () => {
    const css = compiledCss('./lib/components/ItemTooltip.svelte')
    expect(css).toMatch(rule('\\.tooltip', 'user-select: none'))
  })
})

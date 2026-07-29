<script lang="ts">
  import { SvelteMap, SvelteSet } from 'svelte/reactivity'
  import { gameStore } from '../stores/gameStore'
  import { networkManager } from '../network/socket'
  import { handleCommand, visibleCommandNames } from '../chat-commands'
  import { chatInputKeyIntent } from '../chat-input-keys'
  import { chatFocusRequest } from '../stores/npcMenuStore'
  import {
    translationEnabled,
    translationTargetLanguage,
    TRANSLATION_LANGUAGES,
  } from '../stores/translationSettings'
  import {
    translateChatText,
    isTranslatorApiSupported,
  } from '../translation/chatTranslator'

  type Tab = 'say' | 'combat'
  const TRANSCRIPT_FADE_DELAY_MS = 20_000

  let activeTab = $state<Tab>('say')
  let chatMessages = $derived($gameStore.chatMessages)
  let combatMessages = $derived($gameStore.combatMessages)
  let isConnected = $derived($gameStore.isConnected)

  // Translated text per message id, for the active target language only —
  // cleared and retranslated from scratch whenever the target changes. Source
  // is auto-detected per message inside translateChatText.
  let translations = new SvelteMap<number, string>()
  // Ids currently awaiting a translation result — a first-time language pair
  // can mean a model download, so this can stay true for a while.
  let pendingTranslation = new SvelteSet<number>()
  let translatedTarget = ''

  $effect(() => {
    const enabled = $translationEnabled
    const target = $translationTargetLanguage
    // Both arrays share one id counter (see gameStore's nextMessageId), so a
    // single cache keyed by id safely covers chat and combat together.
    const entries = [...chatMessages, ...combatMessages]

    if (!enabled || !isTranslatorApiSupported()) {
      if (translations.size) translations.clear()
      if (pendingTranslation.size) pendingTranslation.clear()
      translatedTarget = ''
      return
    }

    if (target !== translatedTarget) {
      translations.clear()
      pendingTranslation.clear()
      translatedTarget = target
    }

    const liveIds = new Set(entries.map((e) => e.id))
    for (const id of translations.keys()) {
      if (!liveIds.has(id)) translations.delete(id)
    }
    for (const id of pendingTranslation) {
      if (!liveIds.has(id)) pendingTranslation.delete(id)
    }

    for (const entry of entries) {
      if (translations.has(entry.id) || !entry.text) continue
      if (pendingTranslation.has(entry.id)) continue
      pendingTranslation.add(entry.id)
      translateChatText(entry.text, target)
        .then((translated) => translations.set(entry.id, translated))
        .catch(() => {
          // Leave untranslated — the template falls back to the original text.
        })
        .finally(() => pendingTranslation.delete(entry.id))
    }
  })

  function displayText(entry: { id: number; text: string }): string {
    return translations.get(entry.id) ?? entry.text
  }

  function isTranslating(entry: { id: number }): boolean {
    return pendingTranslation.has(entry.id)
  }

  const TRANSLATE_OFF = 'off'

  function handleTranslateLangChange(
    event: Event & { currentTarget: HTMLSelectElement }
  ) {
    const value = event.currentTarget.value
    if (value === TRANSLATE_OFF) {
      translationEnabled.set(false)
    } else {
      translationTargetLanguage.set(value)
      translationEnabled.set(true)
    }
  }
  let messageInput = $state('')
  let chatContainer = $state<HTMLDivElement>()
  let transcriptVisible = $state(true)
  let inputFocused = $state(false)
  let hovering = $state(false)
  let fadeTimer: number | undefined

  let scrollFrame: number | undefined

  // Reading scrollHeight forces a layout flush, so defer it to the next frame
  // and coalesce bursts: a run of combat messages costs one flush, not one each.
  $effect(() => {
    const len =
      activeTab === 'say' ? chatMessages.length : combatMessages.length
    if (!chatContainer || !len || scrollFrame !== undefined) return
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = undefined
      if (chatContainer) chatContainer.scrollTop = chatContainer.scrollHeight
    })
  })

  $effect(() => () => {
    if (scrollFrame !== undefined) cancelAnimationFrame(scrollFrame)
  })

  // Focus and hover hold the transcript open; leaving either restarts the countdown.
  let interacting = $derived(inputFocused || hovering)

  $effect(() => {
    void chatMessages.length
    void combatMessages.length
    void activeTab
    void interacting
    transcriptVisible = true
    window.clearTimeout(fadeTimer)
    if (!interacting) {
      fadeTimer = window.setTimeout(() => {
        transcriptVisible = false
      }, TRANSCRIPT_FADE_DELAY_MS)
    }
    return () => window.clearTimeout(fadeTimer)
  })

  function setHovering(value: boolean) {
    hovering = value
  }

  function sendMessage() {
    const trimmed = messageInput.trim()
    if (!trimmed) return
    if (handleCommand(trimmed)) {
      messageInput = ''
      return
    }
    if (isConnected) {
      networkManager.sendChatMessage(trimmed)
      messageInput = ''
    }
  }

  // Grey preview of what Tab would complete to.
  let commandGhost = $derived.by(() => {
    if (!messageInput.startsWith('/') || messageInput.includes(' ')) return ''
    const match = visibleCommandNames().find(
      (n) => n.startsWith(messageInput) && n !== messageInput
    )
    return match ? match.slice(messageInput.length) : ''
  })

  let tabCycle: { matches: string[]; index: number } | null = null

  function completeCommand() {
    if (!messageInput.startsWith('/') || messageInput.includes(' ')) return
    if (tabCycle && tabCycle.matches[tabCycle.index] === messageInput) {
      tabCycle.index = (tabCycle.index + 1) % tabCycle.matches.length
      messageInput = tabCycle.matches[tabCycle.index]
      return
    }
    const matches = visibleCommandNames().filter((n) =>
      n.startsWith(messageInput)
    )
    if (matches.length === 0) return
    // Skip an exact match so the first Tab lands on what the ghost previews.
    const index = matches[0] === messageInput && matches.length > 1 ? 1 : 0
    tabCycle = { matches, index }
    messageInput = matches[index]
  }

  function handleKeyDown(event: KeyboardEvent) {
    const intent = chatInputKeyIntent(event)
    if (intent === 'complete-command') {
      event.preventDefault()
      completeCommand()
    } else if (intent === 'send') {
      event.preventDefault()
      sendMessage()
    }
  }

  function handleGlobalKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return
    if (event.key === 'Enter' && document.activeElement !== chatInput) {
      event.preventDefault()
      activeTab = 'say'
      chatInput?.focus()
    }
  }

  function restoreViewportAfterKeyboard() {
    for (const delay of [0, 80, 250]) {
      window.setTimeout(() => {
        window.scrollTo(0, 0)
        document.documentElement.scrollTop = 0
        document.body.scrollTop = 0
      }, delay)
    }
  }

  let chatInput = $state<HTMLInputElement>()

  // NPC "Talk" action (click or context menu) asks for input focus.
  $effect(() => {
    if ($chatFocusRequest > 0) {
      activeTab = 'say'
      chatInput?.focus()
    }
  })
</script>

<svelte:window onkeydown={handleGlobalKeydown} />

<!-- Hover only pauses the fade; keyboard users get the same pause via input focus. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="chat-panel"
  class:transcript-faded={!transcriptVisible}
  onmouseenter={() => setHovering(true)}
  onmouseleave={() => setHovering(false)}
>
  <div class="tabs">
    <button
      class="tab"
      class:active={activeTab === 'say'}
      onclick={() => (activeTab = 'say')}
    >
      Chat
    </button>
    <button
      class="tab"
      class:active={activeTab === 'combat'}
      onclick={() => (activeTab = 'combat')}
    >
      Combat
    </button>
  </div>

  <div class="chat-messages" bind:this={chatContainer} role="log">
    {#if activeTab === 'say'}
      {#each chatMessages as entry (entry.id)}
        <div class="message" class:whisper={entry.sender === 'whisper'}>
          {#if entry.name}
            <span
              class="name"
              class:local={entry.sender === 'local'}
              class:remote={entry.sender === 'remote'}>{entry.name}:</span
            >
            {displayText(entry)}
          {:else}
            <span class="system">{displayText(entry)}</span>
          {/if}
          {#if isTranslating(entry)}
            <span class="translating-hint">translating…</span>
          {/if}
        </div>
      {/each}
    {:else}
      {#each combatMessages as entry (entry.id)}
        <div class="message combat">
          {#if entry.name}
            <span
              class="name"
              class:local={entry.sender === 'local'}
              class:remote={entry.sender === 'remote'}>{entry.name}:</span
            >
            <span
              class:hit={entry.hit === true}
              class:miss={entry.hit === false}>{displayText(entry)}</span
            >
          {:else}
            {displayText(entry)}
          {/if}
          {#if isTranslating(entry)}
            <span class="translating-hint">translating…</span>
          {/if}
        </div>
      {/each}
    {/if}
  </div>

  <div class="chat-input" class:disconnected={!isConnected}>
    <div class="input-wrap">
      {#if commandGhost}
        <div class="input-ghost" aria-hidden="true">
          <span class="ghost-typed">{messageInput}</span><span
            class="ghost-suffix">{commandGhost}</span
          >
        </div>
      {/if}
      <input
        type="text"
        bind:this={chatInput}
        bind:value={messageInput}
        onkeydown={handleKeyDown}
        onfocus={() => {
          inputFocused = true
          activeTab = 'say'
        }}
        onblur={() => {
          inputFocused = false
          restoreViewportAfterKeyboard()
        }}
        placeholder="Type a message... (/help for commands)"
        disabled={!isConnected}
      />
    </div>
    {#if isTranslatorApiSupported()}
      <select
        class="translate-lang-select"
        value={$translationEnabled ? $translationTargetLanguage : TRANSLATE_OFF}
        onchange={handleTranslateLangChange}
        title="Translate chat"
      >
        <option value={TRANSLATE_OFF}>Default (Off)</option>
        {#each TRANSLATION_LANGUAGES as lang (lang.code)}
          <option value={lang.code}>{lang.label}</option>
        {/each}
      </select>
    {/if}
    <button
      onclick={sendMessage}
      disabled={!isConnected || !messageInput.trim()}
    >
      Send
    </button>
  </div>
</div>

<style>
  .chat-panel {
    /* Laid out by .bottom-hud (a flex row): take up to 450px on the left but
       shrink toward the action cluster when the viewport is narrow. */
    flex: 0 1 auto;
    min-width: 0;
    width: 450px;
    height: 300px;
    background: rgba(0, 0, 0, 0.8);
    border: 1px solid #4a5568;
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    transition:
      background 700ms ease,
      border-color 700ms ease;
  }

  .chat-panel.transcript-faded {
    background: transparent;
    border-color: transparent;
    pointer-events: none;
  }

  .chat-panel.transcript-faded .chat-input,
  .chat-panel.transcript-faded .chat-input * {
    pointer-events: auto;
  }

  .tabs {
    display: flex;
    border-bottom: 1px solid #4a5568;
    flex-shrink: 0;
    transition: opacity 700ms ease;
  }

  .tab {
    flex: 1;
    padding: 6px 0;
    border: none;
    background: transparent;
    color: #718096;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition:
      color 0.15s,
      background 0.15s;
    border-radius: 8px 8px 0 0;
  }

  .tab:hover {
    color: #e2e8f0;
  }

  .tab.active {
    color: #e2e8f0;
    background: rgba(255, 255, 255, 0.05);
    border-bottom: 2px solid #4299e1;
  }

  .chat-messages {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 10px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 5px;
    width: 100%;
    box-sizing: border-box;
    transition: opacity 700ms ease;
  }

  .chat-panel.transcript-faded .tabs,
  .chat-panel.transcript-faded .chat-messages {
    opacity: 0;
  }

  /* When faded the transcript is see-through, so its clicks must fall through
     to the 3D scene behind it. GameHud's `.game-hud *` rule force-enables
     pointer events on every HUD descendant — including the individual tab
     buttons and message rows — which re-captures those clicks even though the
     containers above are set to none. Override it here (higher specificity) so
     the whole transcript is genuinely click-through. The input bar stays
     interactive via the .chat-input rule above so it can still be tapped. */
  .chat-panel.transcript-faded .tabs,
  .chat-panel.transcript-faded .tabs :global(*),
  .chat-panel.transcript-faded .chat-messages,
  .chat-panel.transcript-faded .chat-messages :global(*) {
    pointer-events: none;
  }

  .message {
    color: #e2e8f0;
    font-size: 12px;
    line-height: 1.4;
    overflow-wrap: break-word;
    text-align: left;
    max-width: 100%;
  }

  .name.local {
    color: #68d391;
    font-weight: 600;
  }

  .name.remote {
    color: #f6e05e;
    font-weight: 600;
  }

  .system {
    color: #a0aec0;
    font-style: italic;
  }

  .translating-hint {
    margin-left: 6px;
    color: #718096;
    font-size: 10px;
    font-style: italic;
    opacity: 0.8;
  }

  .message.whisper {
    color: #b794f4;
  }

  .message.whisper .name {
    font-weight: 600;
  }

  .message.combat {
    color: #f6ad55;
  }

  .hit {
    color: #68d391;
  }

  .miss {
    color: #fc8181;
  }

  .chat-input {
    display: flex;
    gap: 8px;
    border-top: 1px solid #4a5568;
    background: #1a202c;
    border-radius: 0 0 8px 8px;
  }

  .chat-input.disconnected {
    background: #742a2a;
  }

  .input-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
  }

  /* One metrics declaration per breakpoint keeps the ghost aligned with the input. */
  .chat-input input,
  .input-ghost {
    padding: 8px 10px;
    font-size: 12px;
  }

  .chat-input input {
    flex: 1;
    min-width: 0;
    border: none;
    border-radius: 0 0 0 8px;
    background: transparent;
    color: #ffffff;
  }

  /* Grey ghost aligned under the real input: an invisible copy of the typed
     text reserves the caret's width so the suggestion trails it exactly. */
  .input-ghost {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    pointer-events: none;
    overflow: hidden;
  }

  .input-ghost .ghost-typed {
    color: transparent;
    white-space: pre;
  }

  .input-ghost .ghost-suffix {
    color: rgba(113, 128, 150, 0.7);
    white-space: pre;
  }

  .chat-input input:focus {
    outline: none;
    border-color: #4299e1;
  }

  .chat-input input::placeholder {
    color: rgba(113, 128, 150, 0.5);
    font-size: 11px;
    opacity: 1;
  }

  .chat-input input:disabled {
    opacity: 0.5;
  }

  .translate-lang-select {
    margin: 2px 0;
    padding: 0 6px;
    border: none;
    border-radius: 4px;
    background: #2d3748;
    color: #e2e8f0;
    font-size: 11px;
    max-width: 90px;
    cursor: pointer;
  }

  .chat-input button {
    margin: 2px;
    padding: 8px 15px;
    border: none;
    border-radius: 4px;
    background: #4299e1;
    color: #ffffff;
    font-size: 12px;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .chat-input button:hover:not(:disabled) {
    background: #3182ce;
  }

  .chat-input button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .chat-messages::-webkit-scrollbar {
    width: 6px;
  }

  .chat-messages::-webkit-scrollbar-track {
    background: #2d3748;
  }

  .chat-messages::-webkit-scrollbar-thumb {
    background: #4a5568;
    border-radius: 3px;
  }

  /* Phone / narrow: chat stays on the bottom row beside the action cluster and
     just gets narrower (flex-shrink absorbs the squeeze); shrink its height and
     round corners too. */
  @media (max-width: 600px), (pointer: coarse) and (max-width: 900px) {
    .chat-panel {
      height: min(124px, 22dvh);
      box-sizing: border-box;
      border-radius: 6px;
    }

    .tab {
      padding: 4px 0;
      font-size: 10px;
    }

    .chat-messages {
      padding: 5px 6px;
      gap: 2px;
    }

    .message {
      font-size: 10px;
      line-height: 1.25;
    }

    .chat-input {
      gap: 4px;
      border-radius: 0 0 6px 6px;
    }

    .chat-input input,
    .input-ghost {
      padding: 4px 6px;
      font-size: 16px;
      line-height: 1;
    }

    .chat-input input {
      border-radius: 0 0 0 6px;
      min-width: 0;
    }

    .chat-input input::placeholder {
      font-size: 12px;
    }

    .chat-input button {
      margin: 2px;
      padding: 4px 8px;
      font-size: 11px;
    }

    .translate-lang-select {
      max-width: 56px;
      font-size: 10px;
    }
  }
</style>

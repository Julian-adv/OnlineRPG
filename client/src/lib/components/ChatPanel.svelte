<script lang="ts">
  import { untrack } from 'svelte'
  import { SvelteMap, SvelteSet } from 'svelte/reactivity'
  import { gameStore } from '../stores/gameStore'
  import { partyRoster } from '../stores/partyStore'
  import {
    chatChannel,
    shouldRevertToSay,
    type ChatChannel,
  } from '../stores/chatChannelStore'
  import { networkManager } from '../network/socket'
  import { handleCommand, visibleCommandNames } from '../chat-commands'
  import {
    chatInputKeyIntent,
    shouldFocusChatOnEnter,
  } from '../chat-input-keys'
  import { mountOverlay } from '../stores/overlayStack'
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

  type Tab = 'all' | 'party' | 'combat'
  const TRANSCRIPT_FADE_DELAY_MS = 20_000

  let activeTab = $state<Tab>('all')
  let chatMessages = $derived($gameStore.chatMessages)
  // The All tab shows everything; this tab isolates the party channel plus
  // the party/summon system lines (join, leave, disband, calls).
  let partyMessages = $derived(
    $gameStore.chatMessages.filter(
      (e) =>
        e.sender === 'party' ||
        (e.sender === 'system' &&
          (e.text.startsWith('Party:') || e.text.startsWith('Summon:')))
    )
  )
  let combatMessages = $derived($gameStore.combatMessages)
  let isConnected = $derived($gameStore.isConnected)
  let inParty = $derived($partyRoster !== null)

  // Party chat is sticky (a /p holds until /s). Losing the party reverts
  // the input to say only once the draft is empty: silently retargeting a
  // half-typed party line at public chat would leak it to everyone nearby.
  $effect(() => {
    if (shouldRevertToSay(inParty, $chatChannel, messageInput)) {
      chatChannel.set('say')
    }
  })

  let channelMenuOpen = $state(false)

  // Escape is arbitrated through the overlay stack (FPSCounter's handler):
  // with the menu registered, one press closes it and leaves the inventory,
  // fishing session, etc. untouched.
  $effect(() => {
    if (!channelMenuOpen) return
    return mountOverlay('chatChannelMenu', () => (channelMenuOpen = false))
  })

  function selectChannel(channel: ChatChannel) {
    chatChannel.set(channel)
    channelMenuOpen = false
  }

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
    // Combat log is server-generated fixed-format text — chat only. Read
    // outside untrack so new messages rerun the effect.
    const entries = chatMessages

    // The template already tracks the caches for rendering; untrack them here
    // so completed translations don't rerun this effect.
    untrack(() => {
      if (!enabled || !isTranslatorApiSupported()) {
        translations.clear()
        pendingTranslation.clear()
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
        // translateChatText never rejects — failures resolve with the
        // original text, which gets cached so the message isn't retried.
        translateChatText(entry.text, target).then((translated) => {
          // Drop results from a stale target after a language switch.
          if (target !== translatedTarget) return
          translations.set(entry.id, translated)
          pendingTranslation.delete(entry.id)
        })
      }
    })
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
      activeTab === 'all'
        ? chatMessages.length
        : activeTab === 'party'
          ? partyMessages.length
          : combatMessages.length
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
    void partyMessages.length
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
      if ($chatChannel === 'party' && !trimmed.startsWith('/')) {
        networkManager.sendPartyChat(trimmed)
      } else {
        networkManager.sendChatMessage(trimmed)
      }
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

  // Typed lines are invisible on the Combat tab, so focusing the input hops
  // off it; All and Party both show them and keep their place.
  function leaveCombatTab() {
    if (activeTab === 'combat') activeTab = 'all'
  }

  function handleGlobalKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return
    // While typing, Escape is invisible to the overlay-stack handler
    // (it skips input targets), so close the menu here; no double-close.
    if (event.key === 'Escape') {
      if (channelMenuOpen && document.activeElement === chatInput) {
        channelMenuOpen = false
      }
      return
    }
    if (
      shouldFocusChatOnEnter(event, channelMenuOpen) &&
      document.activeElement !== chatInput
    ) {
      event.preventDefault()
      leaveCombatTab()
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

  // NPC "Talk" action (click or context menu) asks for input focus. The
  // counter is consumed, not tested: a stale value must not steal focus on
  // remount (world re-entry, map-editor toggle). The untrack matters too —
  // leaveCombatTab reads activeTab, and tracking it here would rerun this
  // effect on every tab click, bouncing the tab back.
  let seenFocusRequest = $chatFocusRequest
  $effect(() => {
    if ($chatFocusRequest > seenFocusRequest) {
      seenFocusRequest = $chatFocusRequest
      untrack(() => leaveCombatTab())
      chatInput?.focus()
    }
  })
</script>

<svelte:window
  onkeydown={handleGlobalKeydown}
  onclick={() => (channelMenuOpen = false)}
/>

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
      class:active={activeTab === 'all'}
      onclick={() => (activeTab = 'all')}
    >
      All
    </button>
    <button
      class="tab"
      class:active={activeTab === 'party'}
      onclick={() => (activeTab = 'party')}
    >
      Party
    </button>
    <button
      class="tab"
      class:active={activeTab === 'combat'}
      onclick={() => (activeTab = 'combat')}
    >
      Combat
    </button>
  </div>

  <div class="chat-body">
    {#if isTranslatorApiSupported() && activeTab !== 'combat'}
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
    {#snippet chatRow(entry: (typeof chatMessages)[number])}
      <div
        class="message"
        class:whisper={entry.sender === 'whisper'}
        class:party={entry.sender === 'party'}
      >
        {#if entry.name}
          {#if entry.sender === 'party'}
            <span class="party-tag">[Party]</span>
          {/if}
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
    {/snippet}

    <div class="chat-messages" bind:this={chatContainer} role="log">
      {#if activeTab === 'all'}
        {#each chatMessages as entry (entry.id)}
          {@render chatRow(entry)}
        {/each}
      {:else if activeTab === 'party'}
        {#each partyMessages as entry (entry.id)}
          {@render chatRow(entry)}
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
                class:miss={entry.hit === false}>{entry.text}</span
              >
            {:else}
              {entry.text}
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <div class="chat-input" class:disconnected={!isConnected}>
    <div class="channel-wrap">
      {#if channelMenuOpen}
        <div class="channel-menu" role="menu">
          <button
            class="channel-item"
            role="menuitemradio"
            aria-checked={$chatChannel === 'say'}
            onclick={() => selectChannel('say')}
          >
            Say
            {#if $chatChannel === 'say'}<span class="check">✓</span>{/if}
          </button>
          <button
            class="channel-item"
            role="menuitemradio"
            aria-checked={$chatChannel === 'party'}
            disabled={!inParty}
            title={inParty ? undefined : 'Join a party to use party chat'}
            onclick={() => selectChannel('party')}
          >
            Party
            {#if $chatChannel === 'party'}<span class="check">✓</span>{/if}
          </button>
        </div>
      {/if}
      <button
        class="channel-btn"
        aria-haspopup="menu"
        aria-expanded={channelMenuOpen}
        title="Choose where your messages go (/p and /s)"
        onclick={(e) => {
          e.stopPropagation()
          channelMenuOpen = !channelMenuOpen
        }}
      >
        {$chatChannel === 'party' ? 'Party' : 'Say'}
        <span class="caret" aria-hidden="true">▴</span>
      </button>
    </div>
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
          leaveCombatTab()
        }}
        onblur={() => {
          inputFocused = false
          restoreViewportAfterKeyboard()
        }}
        placeholder={$chatChannel === 'party'
          ? 'Message your party...'
          : 'Type a message... (/help for commands)'}
        disabled={!isConnected}
      />
    </div>
    <button
      class="send-btn"
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

  .chat-body {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    transition: opacity 700ms ease;
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
  }

  .chat-panel.transcript-faded .tabs,
  .chat-panel.transcript-faded .chat-body {
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
  .chat-panel.transcript-faded .chat-body,
  .chat-panel.transcript-faded .chat-body :global(*) {
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

  /* Party channel: the PartyPanel's blue, distinct from local green, remote
     yellow, whisper purple, combat orange and system grey. */
  .message.party {
    color: #a8d1ff;
  }

  .message.party .name {
    color: #7ec8ff;
    font-weight: 600;
  }

  .party-tag {
    color: #7ec8ff;
    font-weight: 700;
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

  .channel-wrap {
    position: relative;
    display: flex;
    flex: none;
  }

  /* Same neutral look on both channels; the label alone names the target. */
  .channel-btn {
    margin: 2px 0 2px 2px;
    padding: 8px 10px;
    border: none;
    border-radius: 4px;
    background: #2d3748;
    color: #9fb2c3;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    flex: none;
    transition: color 0.15s;
  }

  .channel-btn:hover {
    color: #ffffff;
  }

  .caret {
    margin-left: 3px;
    font-size: 8px;
    opacity: 0.7;
  }

  .channel-menu {
    position: absolute;
    bottom: calc(100% + 6px);
    left: 2px;
    z-index: 5;
    min-width: 96px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: #1a202c;
    border: 1px solid #4a5568;
    border-radius: 6px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
  }

  .channel-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 6px 10px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: #cbd5e0;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }

  .channel-item:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.06);
    color: #ffffff;
  }

  .channel-item:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .check {
    color: #4299e1;
    font-weight: 700;
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
    position: absolute;
    top: 4px;
    right: 6px;
    z-index: 1;
    padding: 2px 6px;
    border: none;
    border-radius: 4px;
    background: rgba(45, 55, 72, 0.9);
    color: #e2e8f0;
    font-size: 11px;
    max-width: 110px;
    cursor: pointer;
  }

  .send-btn {
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

  .send-btn:hover:not(:disabled) {
    background: #3182ce;
  }

  .send-btn:disabled {
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

    .send-btn {
      margin: 2px;
      padding: 4px 8px;
      font-size: 11px;
    }

    .channel-btn {
      margin: 2px 0 2px 2px;
      padding: 4px 6px;
      font-size: 10px;
    }

    .channel-item {
      padding: 4px 8px;
      font-size: 11px;
    }

    .translate-lang-select {
      max-width: 90px;
      font-size: 10px;
    }
  }
</style>

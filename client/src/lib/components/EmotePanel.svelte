<script lang="ts">
  import {
    emotePanelVisible,
    emoteStopRequest,
    localEmoteAnim,
  } from '../stores/emoteStore'
  import { EMOTE_LIST, type EmoteMeta } from '../emote-meta'
  import { gameStore } from '../stores/gameStore'
  import { networkManager } from '../network/socket'
  import { draggablePanel } from '../actions/draggablePanel'

  const visible = $derived($emotePanelVisible)
  const active = $derived($localEmoteAnim)
  // sendChatMessage is a silent no-op while disconnected; grey out like the
  // chat input does instead of eating clicks. Dead players are greyed out
  // too — the server accepts a corpse's /emote, so the client must not
  // offer it as a two-click affordance.
  const usable = $derived(
    $gameStore.isConnected && ($gameStore.currentPlayer?.health ?? 0) > 0
  )

  function play(emote: EmoteMeta) {
    // Clicking the running looping emote stops it, like Escape does.
    if (emote.loops && active === emote.anim) {
      emoteStopRequest.set(true)
      return
    }
    // Same path as typing the command: the server validates and its
    // broadcast starts our animation (see chat-commands `/emote`).
    networkManager.sendChatMessage(`/emote ${emote.anim}`)
  }
</script>

{#if visible}
  <div class="emote-panel" aria-label="Emotes" use:draggablePanel={'emotes'}>
    <div class="panel-header" data-drag-handle>
      <span class="panel-title">Emotes</span>
      <button
        class="close-btn"
        title="Close"
        onclick={() => emotePanelVisible.set(false)}>×</button
      >
    </div>

    <div class="emote-rows">
      {#each EMOTE_LIST as emote (emote.anim)}
        <button
          class="emote-row"
          class:active={active === emote.anim}
          disabled={!usable}
          title={emote.loops && active === emote.anim
            ? 'Stop'
            : `/emote ${emote.anim}`}
          onclick={() => play(emote)}
        >
          <span class="emote-label">{emote.label}</span>
          {#if emote.loops}
            <span class="loop-badge" title="Loops until you move">↻</span>
          {/if}
        </button>
      {/each}
    </div>

    <div class="panel-hint">/emote &lt;name&gt; · dances stop on move</div>
  </div>
{/if}

<style>
  .emote-panel {
    position: fixed;
    right: 16px;
    top: 16%;
    z-index: 40;
    width: 180px;
    display: flex;
    flex-direction: column;
    backdrop-filter: blur(4px);
    padding: 10px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 10px;
    background: rgba(6, 10, 14, 0.88);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    pointer-events: auto;
  }

  .panel-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 8px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.15);
    margin-bottom: 8px;
  }

  .panel-title {
    flex: 1;
    font-size: 14px;
    font-weight: 700;
    color: #8fe08f;
  }

  .close-btn {
    background: none;
    border: none;
    color: #9fb2c3;
    font-size: 18px;
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #fff;
  }

  .emote-rows {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .emote-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 6px;
    color: #cfd9e3;
    font-family: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition:
      background 150ms ease,
      border-color 150ms ease;
  }

  .emote-row:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.1);
    border-color: rgba(255, 255, 255, 0.35);
    color: #fff;
  }

  .emote-row.active {
    border-color: rgba(88, 255, 88, 0.7);
    box-shadow: 0 0 8px rgba(88, 255, 88, 0.35);
    color: #8fe08f;
  }

  .emote-row:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .emote-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .loop-badge {
    color: #7f8f9f;
    font-size: 11px;
    line-height: 1;
  }

  .emote-row.active .loop-badge {
    color: #8fe08f;
  }

  .panel-hint {
    margin-top: 8px;
    padding-top: 6px;
    border-top: 1px solid rgba(255, 255, 255, 0.1);
    color: #7f8f9f;
    font-size: 10px;
    text-align: center;
  }
</style>

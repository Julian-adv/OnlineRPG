<script lang="ts">
  import type { AccountCharacter } from '../network/socket'

  interface Props {
    accountName: string
    characters: AccountCharacter[]
    selectedCharacterId: number | null
    onStartGame: (
      characterId: number
    ) => Promise<{ ok: boolean; message?: string; renameRequired?: boolean }>
    onDeleteCharacter: (
      characterId: number
    ) => Promise<{ ok: boolean; message?: string }>
    onLogout: () => void
  }

  let {
    accountName,
    characters,
    selectedCharacterId,
    onStartGame,
    onDeleteCharacter,
    onLogout,
  }: Props = $props()

  let isStarting = $state(false)
  let isDeleting = $state(false)
  let errorMessage = $state('')
  let selectedCharacter = $derived(
    characters.find((character) => character.id === selectedCharacterId)
  )

  function isBusy() {
    return isStarting || isDeleting
  }

  async function handleStart(characterId?: number) {
    const id = characterId ?? selectedCharacterId
    if (!id || isBusy()) return

    isStarting = true
    errorMessage = ''
    const result = await onStartGame(id)
    isStarting = false

    // A rename-required refusal opens App's dialog instead.
    if (!result.ok && !result.renameRequired) {
      errorMessage = result.message ?? 'Failed to enter game'
    }
  }

  async function handleDelete() {
    if (!selectedCharacterId || isBusy()) return

    const character = characters.find((c) => c.id === selectedCharacterId)
    if (!character) return

    const confirmed = confirm(
      `Are you sure you want to delete "${character.name}"? This cannot be undone.`
    )
    if (!confirmed) return

    isDeleting = true
    errorMessage = ''
    const result = await onDeleteCharacter(selectedCharacterId)
    isDeleting = false

    if (!result.ok) {
      errorMessage = result.message ?? 'Failed to delete character'
    }
  }

  function formatCharacterClass(value: string) {
    return value.charAt(0).toUpperCase() + value.slice(1)
  }
</script>

<!-- UI overlay only — the 3D scene is rendered in the shared Canvas in App.svelte -->
<div class="character-select-overlay">
  <div class="top-bar">
    <h1 class="title">Character Select</h1>
    <p class="account-name">Account: {accountName}</p>
  </div>

  {#if selectedCharacter}
    <div class="mobile-character-info">
      <div class="info-main">
        <span class="info-name">{selectedCharacter.name}</span>
        <span class="info-meta">
          Lv. {selectedCharacter.level}
          {formatCharacterClass(selectedCharacter.class)} · HP {selectedCharacter.max_hp}
        </span>
      </div>

      <div class="info-stats">
        {#each [['STR', selectedCharacter.attributes.str], ['DEX', selectedCharacter.attributes.dex], ['CON', selectedCharacter.attributes.con], ['INT', selectedCharacter.attributes.int], ['WIS', selectedCharacter.attributes.wis], ['CHA', selectedCharacter.attributes.cha]] as stat (stat[0])}
          <div class="info-stat">
            <span>{stat[0]}</span>
            <strong>{stat[1]}</strong>
          </div>
        {/each}
      </div>
    </div>
  {/if}

  <div class="bottom-row">
    <button
      type="button"
      class="secondary"
      onclick={onLogout}
      disabled={isBusy()}
    >
      Back
    </button>
    <button
      type="button"
      class="primary"
      onclick={() => handleStart()}
      disabled={!selectedCharacterId || isBusy()}
    >
      {isStarting ? 'Starting...' : 'Start'}
    </button>
    <button
      type="button"
      class="danger"
      onclick={handleDelete}
      disabled={!selectedCharacterId || isBusy()}
    >
      {isDeleting ? 'Deleting...' : 'Delete'}
    </button>
    {#if errorMessage}
      <div class="error-message">{errorMessage}</div>
    {/if}
  </div>
</div>

<style>
  .character-select-overlay {
    position: fixed;
    inset: 0;
    box-sizing: border-box;
    width: 100%;
    max-width: 100vw;
    height: 100vh;
    height: 100dvh;
    overflow: hidden;
    z-index: 1;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    pointer-events: none;
    color: #edf2f7;
    /* No background — the gradient is rendered behind the shared Canvas in App.svelte */
  }

  .top-bar {
    text-align: center;
    padding: max(24px, calc(env(safe-area-inset-top) + 12px)) 16px 0;
  }

  .title {
    margin: 0;
    font-size: 28px;
    text-shadow: 0 2px 8px rgba(0, 0, 0, 0.6);
  }

  .account-name {
    margin: 6px 0 0;
    color: #9fb0c6;
    font-size: 13px;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.5);
  }

  .mobile-character-info {
    display: none;
  }

  .bottom-row {
    position: fixed;
    bottom: max(16px, calc(env(safe-area-inset-bottom) + 10px));
    left: 16px;
    right: 60px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    pointer-events: auto;
  }

  .bottom-row button {
    box-sizing: border-box;
    height: 36px;
    border-radius: 7px;
    padding: 0 16px;
    font-size: 14px;
    line-height: 1.2;
    cursor: pointer;
  }

  .bottom-row button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .primary {
    border: none;
    background: #2c7be5;
    color: white;
    font-weight: 600;
  }

  .secondary {
    border: 1px solid #61738a;
    background: #1c2736;
    color: #dbe6f2;
  }

  .danger {
    border: 1px solid #b04040;
    background: #3a1a1a;
    color: #ffa0a0;
  }

  .error-message {
    position: absolute;
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%);
    margin-bottom: 10px;
    border: 1px solid #f28b8b;
    border-radius: 7px;
    padding: 10px 12px;
    background: rgba(175, 45, 45, 0.2);
    color: #ffd2d2;
    font-size: 13px;
    max-width: 400px;
    text-align: center;
    white-space: nowrap;
  }

  @media (max-width: 600px), (max-height: 700px) {
    .top-bar {
      padding-top: max(14px, calc(env(safe-area-inset-top) + 8px));
    }

    .title {
      font-size: 22px;
    }

    .account-name {
      margin-top: 3px;
      font-size: 12px;
    }

    .bottom-row {
      bottom: max(16px, calc(env(safe-area-inset-bottom) + 10px));
      left: 10px;
      right: 60px;
      gap: 8px;
    }

    .bottom-row button {
      height: 36px;
      padding: 0 12px;
      font-size: 13px;
    }

    .mobile-character-info {
      position: fixed;
      left: 60px;
      right: 60px;
      bottom: max(64px, calc(env(safe-area-inset-bottom) + 58px));
      box-sizing: border-box;
      display: grid;
      gap: 8px;
      padding: 10px 12px;
      border: 1px solid rgba(124, 201, 255, 0.7);
      border-radius: 8px;
      background: rgba(16, 25, 38, 0.88);
      box-shadow: 0 6px 18px rgba(0, 0, 0, 0.35);
      pointer-events: auto;
      backdrop-filter: blur(4px);
    }

    .info-main {
      min-width: 0;
      display: grid;
      gap: 2px;
      text-align: center;
    }

    .info-name {
      overflow: hidden;
      color: #f7fafc;
      font-size: 15px;
      font-weight: 700;
      line-height: 1.2;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .info-meta {
      color: #f0c040;
      font-size: 12px;
      line-height: 1.25;
    }

    .info-stats {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 5px;
    }

    .info-stat {
      min-width: 0;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 4px;
      padding: 4px 6px;
      border: 1px solid rgba(83, 101, 123, 0.75);
      border-radius: 6px;
      background: rgba(34, 53, 82, 0.72);
      color: #a7b7ca;
      font-size: 11px;
      line-height: 1.2;
    }

    .info-stat strong {
      color: #e4ecf5;
      font-size: 12px;
    }
  }
</style>

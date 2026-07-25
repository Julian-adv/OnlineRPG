<script lang="ts">
  import { onMount } from 'svelte'
  import { Canvas } from '@threlte/core'
  import GameScene from './lib/components/GameScene.svelte'
  import GameHud from './lib/components/GameHud.svelte'
  import LoginScreen from './lib/components/LoginScreen.svelte'
  import CharacterSelectScreen from './lib/components/CharacterSelectScreen.svelte'
  import CharacterSelectScene from './lib/components/CharacterSelectScene.svelte'
  import CharacterCreateScreen from './lib/components/CharacterCreateScreen.svelte'
  import CharacterCreateScene from './lib/components/CharacterCreateScene.svelte'
  import RenderFrameLimiter from './lib/components/RenderFrameLimiter.svelte'
  import { gameStore } from './lib/stores/gameStore'
  import { createWebGPURenderer } from './lib/utils/renderer'
  import {
    networkManager,
    type AccountCharacter,
    type CharacterClass,
    type Gender,
  } from './lib/network/socket'
  import { startBgm } from './lib/managers/bgmManager'
  import SettingsPanel from './lib/components/SettingsPanel.svelte'
  import { runGpuBenchmark } from './lib/utils/gpuBenchmark'
  import {
    needsAutoQuality,
    qualityForScore,
    applyAutoQuality,
  } from './lib/stores/graphicsSettings'

  let showSettings = $state(false)

  type AppScreen = 'login' | 'character-select' | 'character-create' | 'game'
  type DeathUiState =
    | 'alive'
    | 'waiting_dying'
    | 'dialog_open'
    | 'dialog_closed'
  let screen = $state<AppScreen>('login')
  let serverUrl = $state('')
  let accountName = $state('')
  let accountCharacters = $state<AccountCharacter[]>([])
  let selectedCharacterId = $state<number | null>(null)
  let selectedCharacter = $derived<AccountCharacter | null>(
    accountCharacters.find(
      (character) => character.id === selectedCharacterId
    ) ?? null
  )
  let isPlayerDead = $state(false)
  let currentPlayerHp = $state<number | null>(null)
  let currentPlayerMaxHp = $state<number | null>(null)
  let currentPlayerLevel = $state<number | null>(null)
  let currentPlayerTotalXp = $state<number | null>(null)
  let deathUiState = $state<DeathUiState>('alive')
  let showRespawnDialog = $derived(deathUiState === 'dialog_open')
  let canReopenRespawnDialog = $derived(
    isPlayerDead && deathUiState === 'dialog_closed'
  )
  let wasPlayerDead = false
  // Whether we've seen the current player alive this game session. Used to
  // tell a live death (alive→dead, play the dying animation first) apart from
  // reconnecting/reloading while already dead (show the dialog at once).
  let hasObservedCurrentPlayerAlive = false
  let isCurrentPlayerLoading = $state(false)
  let isSceneCompiling = $state(true)
  let kickedMessage = $state('')

  // Character create screen state
  let createSelectedClass = $state<CharacterClass>('knight')
  let createSelectedGender = $state<Gender>('male')

  // First launch on this browser: measure the GPU while the login screen is
  // up and pick a preset from it. A stored choice, however it got there,
  // always wins, so this runs at most once.
  let gpuProbePending = $state(needsAutoQuality())

  // Whether the shared Canvas should be mounted (all screens except login).
  // Also held back until the probe settles: `antialias` is baked in at
  // `new WebGPURenderer()`, so a result arriving after mount could not be
  // applied and would raise "restart required" on a first launch. The probe
  // caps itself at 3s and normally finishes long before login completes.
  let showCanvas = $derived(screen !== 'login' && !gpuProbePending)

  onMount(() => {
    if (!gpuProbePending) return
    void runGpuBenchmark()
      .then((result) => {
        if (!result) return
        const level = qualityForScore(result.score)
        console.info(
          `[GpuBenchmark] score=${result.score.toFixed(1)} ` +
            `probe=${result.elapsedMs.toFixed(0)}ms -> ${level}`
        )
        applyAutoQuality(level)
      })
      .finally(() => {
        gpuProbePending = false
      })
  })

  $effect(() => {
    if (selectedCharacterId === null) {
      if (accountCharacters.length > 0) {
        selectedCharacterId = accountCharacters[0].id
      }
      return
    }

    const selectedStillExists = accountCharacters.some(
      (character) => character.id === selectedCharacterId
    )
    if (!selectedStillExists) {
      selectedCharacterId =
        accountCharacters.length > 0 ? accountCharacters[0].id : null
    }
  })

  async function handleLogin(
    url: string,
    googleIdToken: string
  ): Promise<{ ok: boolean; message?: string }> {
    kickedMessage = ''
    const result = await networkManager.requestAuthentication(
      url,
      googleIdToken
    )

    if (result.ok) {
      const characters = result.characters ?? []
      serverUrl = url
      accountName = result.accountName ?? ''
      accountCharacters = characters
      selectedCharacterId = characters.length > 0 ? characters[0].id : null
      screen = 'character-select'
      return { ok: true }
    }

    return result
  }

  async function handleCreateCharacter(
    characterName: string,
    characterClass: CharacterClass,
    gender: Gender
  ) {
    const result = await networkManager.requestCreateCharacter(
      characterName,
      characterClass,
      gender
    )
    if (result.ok && result.character) {
      accountCharacters = [...accountCharacters, result.character]
    }
    return result
  }

  async function handleDeleteCharacter(characterId: number) {
    const result = await networkManager.requestDeleteCharacter(characterId)
    if (result.ok) {
      accountCharacters = accountCharacters.filter((c) => c.id !== characterId)
    }
    return result
  }

  async function handleRollCharacterStats(cls: CharacterClass, gender: Gender) {
    return networkManager.requestRollCharacterStats(cls, gender)
  }

  async function handleStartGame(
    characterId: number
  ): Promise<{ ok: boolean; message?: string }> {
    const result = await networkManager.requestEnterGame(characterId)
    if (result.ok) {
      // Fresh death tracking for the new session so an already-dead character
      // (entered while dead) opens the respawn dialog immediately.
      deathUiState = 'alive'
      isPlayerDead = false
      wasPlayerDead = false
      hasObservedCurrentPlayerAlive = false
      isSceneCompiling = true
      screen = 'game'
    }
    return result
  }

  function handleOpenCreateCharacterScreen() {
    if (accountCharacters.length >= 3) return
    screen = 'character-create'
  }

  function handleCancelCreateCharacter() {
    screen = 'character-select'
  }

  function handleCharacterCreated(characterId: number) {
    selectedCharacterId = characterId
    screen = 'character-select'
  }

  function handleSelectCharacter(characterId: number) {
    selectedCharacterId = characterId
  }

  async function handleBackToCharacterSelect() {
    screen = 'character-select'
    const result = await networkManager.requestReauthenticate()
    if (result.ok) {
      accountCharacters = result.characters ?? []
      if (result.accountName) accountName = result.accountName
      if (accountCharacters.length > 0) {
        const stillExists = accountCharacters.some(
          (c) => c.id === selectedCharacterId
        )
        if (!stillExists) {
          selectedCharacterId = accountCharacters[0].id
        }
      } else {
        selectedCharacterId = null
      }
    } else {
      handleLogoutToLogin()
    }
  }

  function handleLogoutToLogin() {
    networkManager.disconnect()
    networkManager.clearSession()
    accountName = ''
    accountCharacters = []
    selectedCharacterId = null
    screen = 'login'
  }

  function requestRespawn() {
    deathUiState = 'dialog_closed'
    networkManager.requestRespawn()
  }

  function closeRespawnDialog() {
    deathUiState = isPlayerDead ? 'dialog_closed' : 'alive'
  }

  function reopenRespawnDialog() {
    if (!isPlayerDead || deathUiState !== 'dialog_closed') return
    deathUiState = 'dialog_open'
  }

  function handleCurrentPlayerDyingFinished() {
    if (screen !== 'game' || !isPlayerDead || deathUiState !== 'waiting_dying')
      return
    deathUiState = 'dialog_open'
  }

  networkManager.kicked.on((reason) => {
    networkManager.clearSession()
    kickedMessage = reason
    accountName = ''
    accountCharacters = []
    selectedCharacterId = null
    deathUiState = 'alive'
    isPlayerDead = false
    screen = 'login'
  })

  gameStore.subscribe((state) => {
    currentPlayerHp = state.currentPlayer?.health ?? null
    currentPlayerMaxHp = state.currentPlayer?.maxHealth ?? null
    currentPlayerLevel = state.currentPlayer?.level ?? null
    currentPlayerTotalXp = state.currentPlayer?.totalXp ?? null
  })

  // Drive the death UI from a reactive effect (not the store subscription) so
  // it re-evaluates when `screen` flips to 'game' as well: on a reload while
  // dead, JoinSuccess sets health=0 before the screen switches, and there may
  // be no later store update for a subscription to react to.
  $effect(() => {
    const hp = currentPlayerHp
    const inGame = screen === 'game'
    const deadNow = inGame && hp !== null && hp <= 0
    if (deadNow && !wasPlayerDead) {
      // First sighting already dead (reconnected/reloaded while dead): open the
      // respawn dialog right away. A live alive→dead transition still plays the
      // dying animation before the dialog opens.
      deathUiState = hasObservedCurrentPlayerAlive
        ? 'waiting_dying'
        : 'dialog_open'
    }
    if (!deadNow) {
      deathUiState = 'alive'
    }
    if (inGame && hp !== null && hp > 0) {
      hasObservedCurrentPlayerAlive = true
    }
    isPlayerDead = deadNow
    wasPlayerDead = deadNow
  })
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<main onclick={startBgm}>
  <!-- Shared Canvas: one WebGPU device across character select, create, and game.
       Pipelines compiled during character select are reused in game. -->
  {#if showCanvas}
    <div class="canvas-layer" class:dead={screen === 'game' && isPlayerDead}>
      <Canvas renderMode="manual" shadows createRenderer={createWebGPURenderer}>
        <RenderFrameLimiter />
        {#if screen === 'character-select'}
          <CharacterSelectScene
            characters={accountCharacters}
            {selectedCharacterId}
            onSlotClick={(i) => {
              const c = accountCharacters[i]
              if (c) {
                handleSelectCharacter(c.id)
              } else {
                handleOpenCreateCharacterScreen()
              }
            }}
            onSlotDoubleClick={(i) => {
              const c = accountCharacters[i]
              if (c) {
                handleSelectCharacter(c.id)
                handleStartGame(c.id)
              }
            }}
          />
        {:else if screen === 'character-create'}
          <CharacterCreateScene
            characterClass={createSelectedClass}
            gender={createSelectedGender}
          />
        {:else if screen === 'game'}
          <GameScene
            {serverUrl}
            onCurrentPlayerDyingFinished={handleCurrentPlayerDyingFinished}
            bind:isCurrentPlayerLoading
            bind:isSceneCompiling
          />
        {/if}
      </Canvas>
    </div>
  {/if}

  <!-- UI overlays (outside Canvas) -->
  {#if screen === 'game'}
    <GameHud
      {selectedCharacter}
      {currentPlayerLevel}
      {currentPlayerTotalXp}
      {currentPlayerHp}
      {currentPlayerMaxHp}
      {canReopenRespawnDialog}
      {showRespawnDialog}
      {isSceneCompiling}
      {isCurrentPlayerLoading}
      onReopenRespawnDialog={reopenRespawnDialog}
      onBackToCharacterSelect={handleBackToCharacterSelect}
      onRespawn={requestRespawn}
      onCloseRespawnDialog={closeRespawnDialog}
      onOpenSettings={() => (showSettings = true)}
    />
  {:else if screen === 'character-select'}
    <CharacterSelectScreen
      {accountName}
      characters={accountCharacters}
      {selectedCharacterId}
      onStartGame={handleStartGame}
      onDeleteCharacter={handleDeleteCharacter}
      onLogout={handleLogoutToLogin}
    />
  {:else if screen === 'character-create'}
    <CharacterCreateScreen
      {accountName}
      characters={accountCharacters}
      selectedClass={createSelectedClass}
      selectedGender={createSelectedGender}
      onClassChange={(cls) => {
        createSelectedClass = cls
      }}
      onGenderChange={(g) => {
        createSelectedGender = g
      }}
      onRollCharacterStats={handleRollCharacterStats}
      onCreateCharacter={handleCreateCharacter}
      onCharacterCreated={handleCharacterCreated}
      onCancel={handleCancelCreateCharacter}
    />
  {:else}
    <LoginScreen onLogin={handleLogin} {kickedMessage} />
  {/if}

  {#if screen !== 'game'}
    <button
      class="settings-btn-corner"
      class:raised={screen === 'character-create'}
      onclick={() => (showSettings = true)}
      title="Settings"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="512"
        height="512"
        viewBox="0 0 512 512"
        ><path
          fill="currentColor"
          d="M495.9 166.6c3.2 8.7 .5 18.4-6.4 24.6l-43.3 39.4c1.1 8.3 1.7 16.8 1.7 25.4s-.6 17.1-1.7 25.4l43.3 39.4c6.9 6.2 9.6 15.9 6.4 24.6c-4.4 11.9-9.7 23.3-15.8 34.3l-4.7 8.1c-6.6 11-14 21.4-22.1 31.2c-5.9 7.2-15.7 9.6-24.5 6.8l-55.7-17.7c-13.4 10.3-28.2 18.9-44 25.4l-12.5 57.1c-2 9.1-9 16.3-18.2 17.8c-13.8 2.3-28 3.5-42.5 3.5s-28.7-1.2-42.5-3.5c-9.2-1.5-16.2-8.7-18.2-17.8l-12.5-57.1c-15.8-6.5-30.6-15.1-44-25.4l-55.7 17.7c-8.8 2.8-18.6 .3-24.5-6.8c-8.1-9.8-15.5-20.2-22.1-31.2l-4.7-8.1c-6.1-11-11.4-22.4-15.8-34.3c-3.2-8.7-.5-18.4 6.4-24.6l43.3-39.4c-1.1-8.4-1.7-16.9-1.7-25.5s.6-17.1 1.7-25.4l-43.3-39.4c-6.9-6.2-9.6-15.9-6.4-24.6c4.4-11.9 9.7-23.3 15.8-34.3l4.7-8.1c6.6-11 14-21.4 22.1-31.2c5.9-7.2 15.7-9.6 24.5-6.8l55.7 17.7c13.4-10.3 28.2-18.9 44-25.4l12.5-57.1c2-9.1 9-16.3 18.2-17.8C227.3 1.2 241.5 0 256 0s28.7 1.2 42.5 3.5c9.2 1.5 16.2 8.7 18.2 17.8l12.5 57.1c15.8 6.5 30.6 15.1 44 25.4l55.7-17.7c8.8-2.8 18.6-.3 24.5 6.8c8.1 9.8 15.5 20.2 22.1 31.2l4.7 8.1c6.1 11 11.4 22.4 15.8 34.3zM256 336a80 80 0 1 0 0-160a80 80 0 1 0 0 160z"
        /></svg
      >
    </button>
  {/if}

  {#if showSettings}
    <SettingsPanel onClose={() => (showSettings = false)} />
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    overflow: hidden;
    background: #1a1a1a;
  }

  main {
    width: 100%;
    max-width: 100vw;
    height: 100vh;
    height: 100dvh;
    overflow: hidden;
    position: relative;
  }

  .canvas-layer {
    position: absolute;
    inset: 0;
    z-index: 0;
    transition: filter 180ms ease;
  }

  .canvas-layer.dead {
    filter: grayscale(100%);
  }

  .settings-btn-corner {
    position: fixed;
    right: max(16px, calc(env(safe-area-inset-right) + 10px));
    bottom: max(16px, calc(env(safe-area-inset-bottom) + 10px));
    box-sizing: border-box;
    width: 36px;
    height: 36px;
    z-index: 9999;
    background: rgba(60, 60, 60, 0.85);
    color: #ccc;
    border: none;
    border-radius: 8px;
    padding: 8px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  .settings-btn-corner:hover {
    background: rgba(80, 80, 80, 0.95);
    color: #fff;
  }

  .settings-btn-corner.raised {
    bottom: max(80px, calc(env(safe-area-inset-bottom) + 80px));
  }

  .settings-btn-corner svg {
    width: 20px;
    height: 20px;
  }
</style>

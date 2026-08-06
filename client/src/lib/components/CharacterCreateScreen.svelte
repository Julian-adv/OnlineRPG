<script lang="ts">
  import type {
    AccountCharacter,
    CharacterClass,
    CharacterRollResult,
    Gender,
    RollCharacterStatsResult,
  } from '../network/socket'
  import { getAvailableGenders } from '../utils/modelPaths'

  const MAX_CHARACTER_SLOTS = 3

  interface Props {
    accountName: string
    characters: AccountCharacter[]
    selectedClass: CharacterClass
    selectedGender: Gender
    onClassChange: (cls: CharacterClass) => void
    onGenderChange: (gender: Gender) => void
    onRollCharacterStats: (
      cls: CharacterClass,
      gender: Gender
    ) => Promise<RollCharacterStatsResult>
    onCreateCharacter: (
      characterName: string,
      characterClass: CharacterClass,
      gender: Gender
    ) => Promise<{
      ok: boolean
      message?: string
      character?: AccountCharacter
    }>
    onCharacterCreated: (characterId: number) => void
    onCancel: () => void
  }

  let {
    accountName,
    characters,
    selectedClass,
    selectedGender,
    onClassChange,
    onGenderChange,
    onRollCharacterStats,
    onCreateCharacter,
    onCharacterCreated,
    onCancel,
  }: Props = $props()

  let availableGenders = $derived(getAvailableGenders(selectedClass))
  let createCharacterName = $state('')
  let rolledStats = $state<CharacterRollResult | null>(null)
  let isCreating = $state(false)
  let isRolling = $state(false)
  let errorMessage = $state('')

  function isBusy() {
    return isCreating || isRolling
  }

  function atSlotLimit() {
    return characters.length >= MAX_CHARACTER_SLOTS
  }

  function selectClass(cls: CharacterClass) {
    onClassChange(cls)
    const genders = getAvailableGenders(cls)
    if (!genders.includes(selectedGender)) {
      onGenderChange(genders[0])
    }
    rolledStats = null
  }

  function selectGender(g: Gender) {
    onGenderChange(g)
    rolledStats = null
  }

  async function handleRoll() {
    if (isBusy()) return
    if (atSlotLimit()) {
      errorMessage = 'A maximum of 3 characters can be created.'
      return
    }

    isRolling = true
    errorMessage = ''
    const result = await onRollCharacterStats(selectedClass, selectedGender)
    isRolling = false

    if (!result.ok) {
      errorMessage = result.message
      return
    }

    rolledStats = { attributes: result.attributes, maxHp: result.maxHp }
  }

  async function submitCreateCharacter(event: Event) {
    event.preventDefault()
    if (isBusy()) return

    if (atSlotLimit()) {
      errorMessage = 'A maximum of 3 characters can be created.'
      return
    }

    const characterName = createCharacterName.trim()
    if (!characterName) {
      errorMessage = 'Please enter character name'
      return
    }
    if (!rolledStats) {
      errorMessage = 'Roll attributes first'
      return
    }

    isCreating = true
    errorMessage = ''
    const result = await onCreateCharacter(
      characterName,
      selectedClass,
      selectedGender
    )
    isCreating = false

    if (!result.ok) {
      errorMessage = result.message ?? 'Failed to create character'
      return
    }

    if (!result.character) {
      errorMessage = 'Character created but no character data returned'
      return
    }

    createCharacterName = ''
    rolledStats = null
    onCharacterCreated(result.character.id)
  }
</script>

<!-- UI overlay only — the 3D scene is rendered in the shared Canvas in App.svelte -->
<div class="character-create-overlay">
  <div class="top-bar">
    <h1 class="title">Create Character</h1>
    <p class="account-name">Account: {accountName}</p>
  </div>

  <form class="create-form" onsubmit={submitCreateCharacter}>
    <div class="class-column">
      <span class="field-label">Class</span>
      <button
        type="button"
        class="class-btn"
        class:class-selected={selectedClass === 'knight'}
        disabled={isBusy()}
        onclick={() => selectClass('knight')}
      >
        Knight
      </button>
      <button
        type="button"
        class="class-btn"
        class:class-selected={selectedClass === 'barbarian'}
        disabled={isBusy()}
        onclick={() => selectClass('barbarian')}
      >
        Barbarian
      </button>
      <button
        type="button"
        class="class-btn"
        class:class-selected={selectedClass === 'rogue'}
        disabled={isBusy()}
        onclick={() => selectClass('rogue')}
      >
        Rogue
      </button>
      <button
        type="button"
        class="class-btn"
        class:class-selected={selectedClass === 'caveman'}
        disabled={isBusy()}
        onclick={() => selectClass('caveman')}
      >
        {selectedGender === 'female' ? 'Cavewoman' : 'Caveman'}
      </button>
      <button
        type="button"
        class="class-btn"
        class:class-selected={selectedClass === 'valkyrie'}
        disabled={isBusy()}
        onclick={() => selectClass('valkyrie')}
      >
        Valkyrie
      </button>
      <button
        type="button"
        class="class-btn"
        class:class-selected={selectedClass === 'ranger'}
        disabled={isBusy()}
        onclick={() => selectClass('ranger')}
      >
        Ranger
      </button>
      <button
        type="button"
        class="class-btn"
        class:class-selected={selectedClass === 'priest'}
        disabled={isBusy()}
        onclick={() => selectClass('priest')}
      >
        Priest
      </button>
      <button
        type="button"
        class="class-btn"
        class:class-selected={selectedClass === 'bard'}
        disabled={isBusy()}
        onclick={() => selectClass('bard')}
      >
        Bard
      </button>
    </div>

    <div class="bottom-bar">
      {#if errorMessage}
        <div class="error-message">{errorMessage}</div>
      {/if}

      <div class="bottom-row">
        <div class="gender-field">
          <span class="field-label">Gender</span>
          <div class="gender-buttons">
            <button
              type="button"
              class="class-btn"
              class:class-selected={selectedGender === 'male'}
              disabled={isBusy() || !availableGenders.includes('male')}
              onclick={() => selectGender('male')}
            >
              Male
            </button>
            <button
              type="button"
              class="class-btn"
              class:class-selected={selectedGender === 'female'}
              disabled={isBusy() || !availableGenders.includes('female')}
              onclick={() => selectGender('female')}
            >
              Female
            </button>
          </div>
        </div>

        <label class="name-field" for="characterName">
          <span class="field-label">Name</span>
          <input
            id="characterName"
            type="text"
            bind:value={createCharacterName}
            maxlength={24}
            placeholder="Enter character name"
            disabled={isBusy()}
          />
        </label>

        <div
          class="rolled-attributes"
          role="button"
          tabindex="0"
          onclick={handleRoll}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') handleRoll()
          }}
        >
          {#if rolledStats}
            <div class="attr">STR {rolledStats.attributes.str}</div>
            <div class="attr">DEX {rolledStats.attributes.dex}</div>
            <div class="attr">CON {rolledStats.attributes.con}</div>
            <div class="attr">INT {rolledStats.attributes.int}</div>
            <div class="attr">WIS {rolledStats.attributes.wis}</div>
            <div class="attr">CHA {rolledStats.attributes.cha}</div>
            <div class="attr">HP {rolledStats.maxHp}</div>
          {:else}
            <div class="roll-hint">
              Roll to generate attributes (4d6 drop lowest, total 72)
            </div>
          {/if}
        </div>

        <div class="create-actions">
          <button
            type="button"
            class="secondary"
            disabled={isBusy()}
            onclick={handleRoll}
          >
            {isRolling ? 'Rolling...' : 'Roll'}
          </button>
          <button
            type="submit"
            class="primary"
            disabled={isBusy() || !rolledStats || atSlotLimit()}
          >
            {isCreating ? 'Creating...' : 'Create'}
          </button>
          <button
            type="button"
            class="secondary"
            disabled={isBusy()}
            onclick={onCancel}
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  </form>
</div>

<style>
  .character-create-overlay {
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
    color: #edf2f7;
    pointer-events: none;
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

  .create-form {
    position: fixed;
    left: 16px;
    bottom: max(16px, calc(env(safe-area-inset-bottom) + 10px));
    max-width: calc(100vw - 72px);
    max-height: calc(100dvh - 92px);
    display: flex;
    align-items: flex-end;
    gap: 10px;
    pointer-events: auto;
  }

  .field-label {
    font-size: 13px;
    color: #b8c6d9;
  }

  .class-column {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 120px;
  }

  .bottom-bar {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .bottom-row {
    display: flex;
    align-items: end;
    gap: 10px;
  }

  .gender-field {
    display: grid;
    gap: 6px;
    min-width: 160px;
  }

  .gender-buttons {
    display: flex;
    gap: 6px;
  }

  .gender-buttons .class-btn {
    height: 34px;
    padding: 6px 12px;
  }

  .class-btn {
    flex: 1;
    border: 1px solid #526276;
    border-radius: 7px;
    padding: 10px 12px;
    background: #111923;
    color: #9fb0c6;
    font-size: 14px;
    cursor: pointer;
    transition:
      background 120ms ease,
      color 120ms ease,
      border-color 120ms ease;
  }

  .class-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .class-btn.class-selected {
    border-color: #2c7be5;
    background: #162a44;
    color: #edf2f7;
    font-weight: 600;
  }

  .name-field {
    flex: 1;
    display: grid;
    gap: 6px;
  }

  .name-field input {
    border: 1px solid #526276;
    border-radius: 7px;
    height: 34px;
    padding: 6px 12px;
    background: #111923;
    color: #f7fafc;
    font-size: 14px;
    box-sizing: border-box;
  }

  .rolled-attributes {
    width: 220px;
    border: 1px solid #45556b;
    border-radius: 8px;
    background: rgba(16, 24, 35, 0.9);
    padding: 10px;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
    height: 90px;
    align-items: center;
    cursor: pointer;
  }

  .attr {
    font-size: 13px;
    font-weight: 600;
    color: #e4ecf5;
    text-align: center;
  }

  .roll-hint {
    grid-column: 1 / -1;
    font-size: 12px;
    color: #9fb0c6;
    text-align: center;
  }

  .create-actions {
    display: flex;
    align-items: end;
    gap: 10px;
    margin-left: auto;
  }

  .create-actions button {
    flex: 1;
    border-radius: 7px;
    height: 34px;
    padding: 6px 12px;
    font-size: 14px;
    line-height: 1.2;
    cursor: pointer;
  }

  .create-actions button:disabled {
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

  .error-message {
    border: 1px solid #f28b8b;
    border-radius: 7px;
    padding: 10px 12px;
    background: rgba(175, 45, 45, 0.2);
    color: #ffd2d2;
    font-size: 13px;
    text-align: center;
  }

  @media (max-width: 1100px) {
    .bottom-row {
      flex-direction: column;
    }

    .create-actions {
      margin-left: 0;
    }
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

    .create-form {
      left: 10px;
      bottom: max(10px, calc(env(safe-area-inset-bottom) + 8px));
      max-width: calc(100vw - 62px);
      max-height: calc(100dvh - 68px);
      gap: 8px;
    }

    .field-label {
      font-size: 12px;
    }

    .class-column {
      gap: 4px;
      min-width: 104px;
    }

    .class-btn {
      padding: 7px 9px;
      font-size: 13px;
      line-height: 1.15;
    }

    .bottom-bar {
      gap: 8px;
    }

    .bottom-row {
      gap: 8px;
    }

    .gender-field {
      gap: 4px;
      min-width: 134px;
    }

    .gender-buttons {
      gap: 5px;
    }

    .gender-buttons .class-btn,
    .name-field input,
    .create-actions button {
      height: 30px;
    }

    .name-field {
      gap: 4px;
    }

    .name-field input {
      padding: 5px 9px;
      font-size: 13px;
    }

    .rolled-attributes {
      width: 180px;
      height: 72px;
      padding: 8px;
      gap: 4px;
    }

    .attr {
      font-size: 12px;
    }

    .roll-hint {
      font-size: 11px;
    }

    .create-actions {
      gap: 6px;
    }

    .create-actions button {
      padding: 5px 9px;
      font-size: 13px;
    }
  }

  @media (max-height: 560px) {
    .top-bar {
      padding-top: max(8px, env(safe-area-inset-top));
    }

    .title {
      font-size: 18px;
    }

    .account-name {
      display: none;
    }

    .class-column {
      gap: 3px;
    }

    .class-btn {
      padding-top: 6px;
      padding-bottom: 6px;
    }
  }
</style>

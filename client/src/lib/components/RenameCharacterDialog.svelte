<script lang="ts">
  interface Props {
    onRename: (newName: string) => Promise<{ ok: boolean; message?: string }>
    onCancel: () => void
  }

  let { onRename, onCancel }: Props = $props()

  let newName = $state('')
  let errorMessage = $state('')
  let isRenaming = $state(false)

  async function submit(event: Event) {
    event.preventDefault()
    if (isRenaming) return

    const trimmed = newName.trim()
    if (!trimmed) {
      errorMessage = 'Please enter a new name'
      return
    }

    isRenaming = true
    errorMessage = ''
    const result = await onRename(trimmed)
    isRenaming = false

    if (!result.ok) {
      errorMessage = result.message ?? 'Failed to rename character'
    }
  }
</script>

<div class="backdrop">
  <form class="dialog" onsubmit={submit}>
    <h2>New name required</h2>
    <p>This name can no longer be used. Please choose a new one.</p>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      type="text"
      bind:value={newName}
      maxlength="24"
      placeholder="New character name"
      disabled={isRenaming}
      autofocus
    />
    {#if errorMessage}
      <div class="error-message">{errorMessage}</div>
    {/if}
    <div class="actions">
      <button
        type="button"
        class="secondary"
        onclick={onCancel}
        disabled={isRenaming}
      >
        Cancel
      </button>
      <button type="submit" class="primary" disabled={isRenaming}>
        {isRenaming ? 'Renaming...' : 'Rename'}
      </button>
    </div>
  </form>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(6, 10, 16, 0.7);
    color: #edf2f7;
  }

  .dialog {
    width: min(360px, calc(100vw - 32px));
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    border: 1px solid #61738a;
    border-radius: 10px;
    background: #16202c;
  }

  h2 {
    margin: 0;
    font-size: 18px;
  }

  p {
    margin: 0;
    font-size: 13px;
    color: #9fb0c6;
  }

  input {
    box-sizing: border-box;
    height: 36px;
    padding: 0 10px;
    border: 1px solid #61738a;
    border-radius: 7px;
    background: #0f1720;
    color: #edf2f7;
    font-size: 14px;
  }

  .error-message {
    font-size: 13px;
    color: #ff8686;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .actions button {
    box-sizing: border-box;
    height: 36px;
    border-radius: 7px;
    padding: 0 16px;
    font-size: 14px;
    cursor: pointer;
  }

  .actions button:disabled {
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
</style>

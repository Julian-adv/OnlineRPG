<script lang="ts">
  import type { Writable } from 'svelte/store'

  interface Props {
    id: string
    label: string
    volume: Writable<number>
    muted: Writable<boolean>
    shortcut?: string
  }

  let { id, label, volume, muted, shortcut }: Props = $props()
  const muteTitle = $derived(
    ($muted ? 'Unmute' : 'Mute') + (shortcut ? ` (${shortcut})` : '')
  )

  function handleChange(e: Event) {
    const target = e.target as HTMLInputElement
    volume.set(parseFloat(target.value))
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault()
    const delta = e.deltaY < 0 ? 0.01 : -0.01
    volume.update(
      (v) => Math.round(Math.max(0, Math.min(1, v + delta)) * 100) / 100
    )
  }
</script>

<label for={id}>{label}</label>
<div class="slider-row">
  <button
    class="mute-btn"
    class:muted={$muted}
    onclick={() => muted.update((m) => !m)}
    title={muteTitle}
  >
    {#if $muted}
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="16"
        height="16"
        viewBox="0 0 576 512"
        ><path
          fill="currentColor"
          d="M301.1 34.8C312.6 40 320 51.4 320 64v384c0 12.6-7.4 24-18.9 29.2s-25 3.1-34.4-5.3L131.8 352H64c-35.3 0-64-28.7-64-64v-64c0-35.3 28.7-64 64-64h67.8L266.7 40.1c9.4-8.4 22.9-10.4 34.4-5.3zM425 167l55 55 55-55c9.4-9.4 24.6-9.4 33.9 0s9.4 24.6 0 33.9l-55 55 55 55c9.4 9.4 9.4 24.6 0 33.9s-24.6 9.4-33.9 0l-55-55-55 55c-9.4 9.4-24.6 9.4-33.9 0s-9.4-24.6 0-33.9l55-55-55-55c-9.4-9.4-9.4-24.6 0-33.9s24.6-9.4 33.9 0z"
        /></svg
      >
    {:else}
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="16"
        height="16"
        viewBox="0 0 448 512"
        ><path
          fill="currentColor"
          d="M301.1 34.8C312.6 40 320 51.4 320 64v384c0 12.6-7.4 24-18.9 29.2s-25 3.1-34.4-5.3L131.8 352H64c-35.3 0-64-28.7-64-64v-64c0-35.3 28.7-64 64-64h67.8L266.7 40.1c9.4-8.4 22.9-10.4 34.4-5.3zM412.6 181.5C434.1 199.1 448 225.9 448 256s-13.9 56.9-35.4 74.5c-10.3 8.4-25.4 6.8-33.8-3.5s-6.8-25.4 3.5-33.8C393.1 284.4 400 271 400 256s-6.9-28.4-17.7-37.3c-10.3-8.4-11.8-23.5-3.5-33.8s23.5-11.8 33.8-3.5z"
        /></svg
      >
    {/if}
  </button>
  <input
    type="range"
    {id}
    min="0"
    max="1"
    step="0.01"
    value={$volume}
    oninput={handleChange}
    onwheel={handleWheel}
    disabled={$muted}
  />
  <span class="volume-value"
    >{$muted ? 'MUTE' : `${Math.round($volume * 100)}%`}</span
  >
</div>

<style>
  label {
    color: #a0aec0;
    font-size: 13px;
    font-weight: 500;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .slider-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .mute-btn {
    background: none;
    border: none;
    color: #a0aec0;
    cursor: pointer;
    padding: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    flex-shrink: 0;
    transition: color 150ms ease;
  }

  .mute-btn:hover {
    color: #fff;
  }

  .mute-btn.muted {
    color: #fc8181;
  }

  input[type='range'] {
    flex: 1;
    accent-color: #4299e1;
    height: 6px;
  }

  input[type='range']:disabled {
    opacity: 0.3;
  }

  .volume-value {
    color: #edf2f7;
    font-size: 13px;
    font-family: 'Courier New', monospace;
    min-width: 36px;
    text-align: right;
  }
</style>

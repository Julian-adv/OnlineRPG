<script lang="ts">
  import type { Writable } from 'svelte/store'
  import { bgmVolume, bgmMuted } from '../managers/bgmManager'
  import { sfxVolume, sfxMuted } from '../managers/sfxManager'
  import {
    graphicsQuality,
    reloadNeeded,
    setQualityManual,
    type QualityLevel,
  } from '../stores/graphicsSettings'
  import VolumeControl from './VolumeControl.svelte'
  import { minimapEnabled } from '../stores/minimapStore'
  import { alwaysRun } from '../stores/movementSettings'
  import ToggleSwitch from './ToggleSwitch.svelte'
  import { friendOnlineNoticeEnabled } from '../stores/friendStore'
  import { mountOverlay } from '../stores/overlayStack'
  import { resetPanelLayout } from '../stores/panelLayout'
  import { titleLanguage, TITLE_LANGUAGES } from '../data/titleDefs'

  interface Props {
    onClose: () => void
  }

  let { onClose }: Props = $props()

  // Escape closes via the overlay stack, same as the close button.
  $effect(() => mountOverlay('settings', onClose))

  const qualityOptions: { value: QualityLevel; label: string }[] = [
    { value: 'high', label: 'High' },
    { value: 'medium', label: 'Medium' },
    { value: 'low', label: 'Low' },
  ]
</script>

{#snippet toggleRow(label: string, checked: Writable<boolean>, hint?: string)}
  <div class="setting-row">
    <span class="setting-label">
      {label}
      {#if hint}<span class="setting-hint">{hint}</span>{/if}
    </span>
    <ToggleSwitch {checked} {label} />
  </div>
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" onclick={onClose}>
  <div class="panel" onclick={(e) => e.stopPropagation()}>
    <div class="header">
      <h2>Settings</h2>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    {@render toggleRow('Minimap', minimapEnabled)}
    {@render toggleRow(
      'Always Run',
      alwaysRun,
      $alwaysRun ? 'Hold Shift to walk' : 'Hold Shift to run'
    )}
    {@render toggleRow('Friend Online Notice', friendOnlineNoticeEnabled)}

    <div class="setting-row">
      <span class="setting-label">Graphics Quality</span>
      <div class="quality-row">
        {#each qualityOptions as opt (opt.value)}
          <button
            class="quality-btn"
            class:active={$graphicsQuality === opt.value}
            onclick={() => setQualityManual(opt.value)}
          >
            {opt.label}
          </button>
        {/each}
      </div>
    </div>
    {#if $reloadNeeded}
      <div class="reload-notice">
        <span>Antialiasing changes require restart</span>
        <button class="action-btn" onclick={() => location.reload()}
          >Restart</button
        >
      </div>
    {/if}

    <div class="setting-row">
      <span class="setting-label">
        Title Language
        <span class="setting-hint">Auto follows the browser</span>
      </span>
      <div class="quality-row">
        {#each TITLE_LANGUAGES as opt (opt.value)}
          <button
            class="quality-btn"
            class:active={$titleLanguage === opt.value}
            onclick={() => titleLanguage.set(opt.value)}
          >
            {opt.label}
          </button>
        {/each}
      </div>
    </div>

    <div class="setting-row">
      <span class="setting-label">
        UI Layout
        <span class="setting-hint">Panel positions</span>
      </span>
      <button class="action-btn" onclick={resetPanelLayout}>Reset</button>
    </div>

    <div class="divider"></div>

    <div class="volume-row">
      <VolumeControl
        id="bgm-volume"
        label="BGM Volume"
        volume={bgmVolume}
        muted={bgmMuted}
        shortcut="Ctrl+M"
      />
    </div>

    <div class="volume-row sfx-row">
      <VolumeControl
        id="sfx-volume"
        label="Sound Effects"
        volume={sfxVolume}
        muted={sfxMuted}
      />
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 10000;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .panel {
    background: #1a202c;
    border: 1px solid #4a5568;
    border-radius: 10px;
    padding: 24px;
    width: 320px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  }

  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 20px;
  }

  h2 {
    margin: 0;
    color: #edf2f7;
    font-size: 18px;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .close-btn {
    background: none;
    border: none;
    color: #a0aec0;
    font-size: 24px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #fff;
  }

  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-height: 30px;
  }

  .setting-row + .setting-row,
  .reload-notice + .setting-row {
    margin-top: 12px;
  }

  .volume-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .sfx-row {
    margin-top: 16px;
  }

  .divider {
    height: 1px;
    background: #2d3748;
    margin: 16px 0;
  }

  .setting-label {
    color: #a0aec0;
    font-size: 13px;
    font-weight: 500;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .setting-hint {
    display: block;
    color: #718096;
    font-size: 11px;
    font-weight: 400;
    margin-top: 2px;
  }

  .quality-row {
    display: flex;
    gap: 0;
    border-radius: 6px;
    overflow: hidden;
    border: 1px solid #4a5568;
  }

  .quality-btn {
    padding: 5px 12px;
    background: #2d3748;
    color: #a0aec0;
    border: none;
    font-size: 13px;
    font-weight: 500;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    cursor: pointer;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  .quality-btn:not(:last-child) {
    border-right: 1px solid #4a5568;
  }

  .quality-btn:hover {
    background: #374151;
    color: #edf2f7;
  }

  .quality-btn.active {
    background: #4299e1;
    color: #fff;
  }

  .reload-notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 4px;
    padding: 6px 10px;
    background: #2d3748;
    border: 1px solid #4a5568;
    border-radius: 6px;
    font-size: 12px;
    color: #ecc94b;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .action-btn {
    background: #4a5568;
    color: #edf2f7;
    border: none;
    border-radius: 4px;
    padding: 3px 10px;
    font-size: 12px;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    cursor: pointer;
    transition: background 150ms ease;
    flex-shrink: 0;
  }

  .action-btn:hover {
    background: #718096;
  }
</style>

<script lang="ts">
  import ChatPanel from './ChatPanel.svelte'
  import FPSCounter from './FPSCounter.svelte'
  import WavePhaseDebug from './WavePhaseDebug.svelte'
  import GameTimeWidget from './GameTimeWidget.svelte'
  import CelestialDebugDialog from './CelestialDebugDialog.svelte'
  import MapEditorPanel from './map-editor/MapEditorPanel.svelte'
  import HousingEditorPanel from './map-editor/HousingEditorPanel.svelte'
  import CharacterPanel from './CharacterPanel.svelte'
  import InventoryPanel from './InventoryPanel.svelte'
  import QuickslotBar from './QuickslotBar.svelte'
  import TradeWindow from './TradeWindow.svelte'
  import FishingPrompt from './FishingPrompt.svelte'
  import TradeOfferToast from './TradeOfferToast.svelte'
  import NpcContextMenu from './NpcContextMenu.svelte'
  import DragGhost from './DragGhost.svelte'
  import LoadingDialog from './LoadingDialog.svelte'
  import RespawnDialog from './RespawnDialog.svelte'
  import WorldMapDialog from './WorldMapDialog.svelte'
  import ServerNotice from './ServerNotice.svelte'
  import {
    mapEditorMode,
    worldMapVisible,
    inventoryVisible,
    characterPanelVisible,
    teleportLoading,
    housingEditorMode,
  } from '../stores/debugStore'
  import type { AccountCharacter } from '../network/socket'

  interface Props {
    selectedCharacter: AccountCharacter | null
    currentPlayerLevel: number | null
    currentPlayerTotalXp: number | null
    currentPlayerHp: number | null
    currentPlayerMaxHp: number | null
    canReopenRespawnDialog: boolean
    showRespawnDialog: boolean
    isSceneCompiling: boolean
    isCurrentPlayerLoading: boolean
    onReopenRespawnDialog: () => void
    onBackToCharacterSelect: () => void
    onRespawn: () => void
    onCloseRespawnDialog: () => void
    onOpenSettings: () => void
  }

  let {
    selectedCharacter,
    currentPlayerLevel,
    currentPlayerTotalXp,
    currentPlayerHp,
    currentPlayerMaxHp,
    canReopenRespawnDialog,
    showRespawnDialog,
    isSceneCompiling,
    isCurrentPlayerLoading,
    onReopenRespawnDialog,
    onBackToCharacterSelect,
    onRespawn,
    onCloseRespawnDialog,
    onOpenSettings,
  }: Props = $props()
</script>

<div class="game-hud">
  <ServerNotice />
  <FPSCounter />
  <WavePhaseDebug />
  <GameTimeWidget />
  <DragGhost />
  <CelestialDebugDialog />
  {#if $mapEditorMode}
    <MapEditorPanel />
  {/if}
  {#if $housingEditorMode}
    <HousingEditorPanel />
  {/if}
  {#if selectedCharacter && !$mapEditorMode}
    <CharacterPanel
      visible={$characterPanelVisible}
      name={selectedCharacter.name}
      characterClass={selectedCharacter.class}
      gender={selectedCharacter.gender}
      level={currentPlayerLevel ?? selectedCharacter.level}
      currentXp={currentPlayerTotalXp ?? selectedCharacter.xp}
      currentHp={currentPlayerHp ?? selectedCharacter.max_hp}
      maxHp={currentPlayerMaxHp ?? selectedCharacter.max_hp}
      attributes={selectedCharacter.attributes}
      onClose={() => characterPanelVisible.set(false)}
    />
    <InventoryPanel
      visible={$inventoryVisible}
      attributes={selectedCharacter.attributes}
      onClose={() => inventoryVisible.set(false)}
    />
    <TradeWindow />
    <TradeOfferToast />
    <NpcContextMenu />
    <FishingPrompt />
  {/if}

  <div class="bottom-hud">
    {#if !$mapEditorMode}
      <ChatPanel />
    {/if}
    <div class="action-cluster">
      {#if selectedCharacter && !$mapEditorMode}
        <QuickslotBar characterId={selectedCharacter.id} />
      {/if}
      <div class="corner-actions">
        {#if canReopenRespawnDialog}
          <button class="respawn-reopen" onclick={onReopenRespawnDialog}>
            Revive
          </button>
        {/if}
        <button
          class="corner-btn"
          onclick={() => characterPanelVisible.update((v) => !v)}
          title="Character (C)"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="448"
            height="512"
            viewBox="0 0 448 512"
            ><path
              fill="currentColor"
              d="M224 256A128 128 0 1 0 224 0a128 128 0 1 0 0 256zm-45.7 48C79.8 304 0 383.8 0 482.3C0 498.7 13.3 512 29.7 512H418.3c16.4 0 29.7-13.3 29.7-29.7C448 383.8 368.2 304 269.7 304H178.3z"
            /></svg
          >
        </button>
        <button
          class="corner-btn"
          onclick={() => inventoryVisible.update((v) => !v)}
          title="Inventory (I)"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="48"
            height="48"
            viewBox="0 0 48 48"
            ><defs
              ><mask id="SVG1C6FqcGC"
                ><g
                  fill="none"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="4"
                  ><path
                    stroke="#fff"
                    d="M19 9.556V4h-6v10m16-4.444V4h6v10"
                  /><path
                    fill="#fff"
                    stroke="#fff"
                    d="M11 20c0-5.523 4.477-10 10-10h6c5.523 0 10 4.477 10 10v20a4 4 0 0 1-4 4H15a4 4 0 0 1-4-4z"
                  /><path stroke="#fff" d="M11 29H5v10h6m26-10h6v10h-6" /><path
                    stroke="#000"
                    d="M28 23v4m-11-4h14"
                  /></g
                ></mask
              ></defs
            ><path
              fill="currentColor"
              d="M0 0h48v48H0z"
              mask="url(#SVG1C6FqcGC)"
            /></svg
          >
        </button>
        <button
          class="corner-btn"
          onclick={() => worldMapVisible.update((v) => !v)}
          title="World Map (M)"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="576"
            height="512"
            viewBox="0 0 576 512"
            ><path
              fill="currentColor"
              d="M384 476.1L192 421.2V35.9L384 90.8zM416 88.4V456l138.5-69.3c11.9-5.9 21.5-17.4 21.5-30.7V32c0-22-21.5-37.5-42.7-30.7L416 88.4zM160 421.2l-25.5-8.5C94 400.3 64 363.6 64 321.4V280h32c17.7 0 32-14.3 32-32s-14.3-32-32-32H64V192c0-17.7-14.3-32-32-32S0 174.3 0 192v129.4C0 383.5 38.3 439 91.3 457.2l68.7 22.9V88.4L21.2 33.7C9.3 39.6 0 51.1 0 64.4v1.6h32c17.7 0 32 14.3 32 32s-14.3 32-32 32H0v24h64c17.7 0 32 14.3 32 32s-14.3 32-32 32H0v105.4c0 62.1 38.3 117.6 91.3 135.8l68.7 22.9z"
            /></svg
          >
        </button>
        <button
          class="corner-btn"
          onclick={onBackToCharacterSelect}
          title="Character Select"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="640"
            height="512"
            viewBox="0 0 640 512"
            ><path
              fill="currentColor"
              d="M72 88a56 56 0 1 1 112 0a56 56 0 1 1-112 0m-8 157.7c-10 11.2-16 26.1-16 42.3s6 31.1 16 42.3v-84.7zm144.4-49.3C178.7 222.7 160 261.2 160 304c0 34.3 12 65.8 32 90.5V416c0 17.7-14.3 32-32 32H96c-17.7 0-32-14.3-32-32v-26.8C26.2 371.2 0 332.7 0 288c0-61.9 50.1-112 112-112h32c24 0 46.2 7.5 64.4 20.3zM448 416v-21.5c20-24.7 32-56.2 32-90.5c0-42.8-18.7-81.3-48.4-107.7C449.8 183.5 472 176 496 176h32c61.9 0 112 50.1 112 112c0 44.7-26.2 83.2-64 101.2V416c0 17.7-14.3 32-32 32h-64c-17.7 0-32-14.3-32-32m8-328a56 56 0 1 1 112 0a56 56 0 1 1-112 0m120 157.7v84.7c10-11.3 16-26.1 16-42.3s-6-31.1-16-42.3zM320 32a64 64 0 1 1 0 128a64 64 0 1 1 0-128m-80 272c0 16.2 6 31 16 42.3v-84.7c-10 11.3-16 26.1-16 42.3zm144-42.3v84.7c10-11.3 16-26.1 16-42.3s-6-31.1-16-42.3zm64 42.3c0 44.7-26.2 83.2-64 101.2V448c0 17.7-14.3 32-32 32h-64c-17.7 0-32-14.3-32-32v-42.8c-37.8-18-64-56.5-64-101.2c0-61.9 50.1-112 112-112h32c61.9 0 112 50.1 112 112"
            /></svg
          >
        </button>
        <button class="corner-btn" onclick={onOpenSettings} title="Settings">
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
      </div>
    </div>
  </div>
</div>

{#if isSceneCompiling || isCurrentPlayerLoading || $teleportLoading}
  <LoadingDialog
    message={isSceneCompiling ? 'Preparing world...' : 'Loading...'}
  />
{/if}

{#if showRespawnDialog}
  <RespawnDialog {onRespawn} onLater={onCloseRespawnDialog} />
{/if}

{#if $worldMapVisible}
  <WorldMapDialog />
{/if}

<style>
  .game-hud {
    position: absolute;
    inset: 0;
    z-index: 1;
    pointer-events: none;
    /* Corner menu-button dimensions. --menu-block-2row caps the menu's width so
       its five buttons wrap to a 3+2 block on narrow screens. */
    --corner-btn-size: 36px;
    --corner-gap: 8px;
    --menu-block-2row: calc(3 * var(--corner-btn-size) + 2 * var(--corner-gap));
  }

  /* Allow pointer events on interactive HUD children */
  .game-hud :global(*) {
    pointer-events: auto;
  }

  /* Single bottom strip holding chat (left), quickslots + menu (right). The
     wrapper and the action cluster are layout-only, so they stay click-through
     (the gaps between panels must pass clicks down to the 3D scene) while their
     real children re-enable pointer events via the :global(*) rule above. */
  .bottom-hud {
    position: fixed;
    left: 9px;
    right: 9px;
    bottom: 9px;
    z-index: 30;
    display: flex;
    align-items: flex-end;
    gap: 16px;
    pointer-events: none;
  }

  .action-cluster {
    /* Hug the right edge regardless of whether the chat panel is present. */
    margin-left: auto;
    display: flex;
    align-items: flex-end;
    gap: 16px;
    pointer-events: none;
    /* Rigid: the chat panel (flex-shrink:1) absorbs all width changes, so the
       quickslot bar is never squeezed into extra rows when the viewport narrows. */
    flex-shrink: 0;
  }

  .corner-actions {
    display: flex;
    /* Right-aligned wrapping row capped at the two-row block width, so the 5
       menu buttons wrap to a 3+2 block on narrow screens. Collapses to a single
       row at >=1000px below. */
    flex-direction: row;
    flex-wrap: wrap;
    justify-content: flex-end;
    max-width: var(--menu-block-2row);
    align-items: center;
    gap: var(--corner-gap);
  }

  .respawn-reopen,
  .corner-btn {
    border: none;
    border-radius: 8px;
    padding: 8px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .corner-btn svg {
    width: 20px;
    height: 20px;
  }

  .respawn-reopen {
    background: #e2b93b;
    color: #1a1a1a;
    font-weight: 700;
    /* Always sit alone on its own top row so the other buttons keep the
       same 3+2 wrap layout whether or not Revive is present. */
    order: -1;
    flex-basis: 100%;
  }

  .corner-btn {
    background: rgba(60, 60, 60, 0.85);
    color: #ccc;
    font-weight: 600;
    transition:
      background 150ms ease,
      color 150ms ease;
  }

  .corner-btn:hover {
    background: rgba(80, 80, 80, 0.95);
    color: #fff;
  }

  /* Below 1000px the menu wraps to a narrow two-row (3+2) block; at >=1000px
     there is room for the single five-button row. */
  @media (min-width: 1000px) {
    .corner-actions {
      flex-wrap: nowrap;
      max-width: none;
    }
  }

  /* Phone / narrow: keep everything on one row (chat shrinks, it does not wrap
     above the cluster) and respect the safe-area insets. */
  @media (max-width: 600px), (pointer: coarse) and (max-width: 900px) {
    .bottom-hud {
      left: max(9px, env(safe-area-inset-left));
      right: max(9px, env(safe-area-inset-right));
      bottom: max(9px, env(safe-area-inset-bottom));
    }
  }

  @media (max-width: 768px), (max-height: 520px) and (pointer: coarse) {
    /* Smaller buttons and gap on touch/narrow screens; still 3 per row. Set on
       .game-hud so both the menu and the quickslot bar read the same values. */
    .game-hud {
      --corner-btn-size: 32px;
      --corner-gap: 5px;
    }

    .respawn-reopen {
      height: 32px;
      padding: 0 8px;
      font-size: 11px;
    }

    .corner-btn {
      width: 32px;
      height: 32px;
      padding: 7px;
    }

    .corner-btn svg {
      width: 16px;
      height: 16px;
    }
  }

  @media (orientation: landscape) and (pointer: coarse) and (max-height: 600px) {
    .bottom-hud {
      bottom: 2px;
    }
  }
</style>

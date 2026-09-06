<script lang="ts">
  import type { Writable } from 'svelte/store'
  import ChatPanel from './ChatPanel.svelte'
  import FPSCounter from './FPSCounter.svelte'
  import WavePhaseDebug from './WavePhaseDebug.svelte'
  import GameTimeWidget from './GameTimeWidget.svelte'
  import Minimap from './Minimap.svelte'
  import CelestialDebugDialog from './CelestialDebugDialog.svelte'
  import MapEditorPanel from './map-editor/MapEditorPanel.svelte'
  import HousingEditorPanel from './map-editor/HousingEditorPanel.svelte'
  import CharacterPanel from './CharacterPanel.svelte'
  import InventoryPanel from './InventoryPanel.svelte'
  import FencePlacementBar from './FencePlacementBar.svelte'
  import QuickslotBar from './QuickslotBar.svelte'
  import HungerIndicator from './HungerIndicator.svelte'
  import LevelBadge from './LevelBadge.svelte'
  import TradeWindow from './TradeWindow.svelte'
  import FishingPrompt from './FishingPrompt.svelte'
  import TradeOfferToast from './TradeOfferToast.svelte'
  import PartyInviteToast from './PartyInviteToast.svelte'
  import PlayerTradeWindow from './PlayerTradeWindow.svelte'
  import PlayerTradeRequestToast from './PlayerTradeRequestToast.svelte'
  import PartySummonToast from './PartySummonToast.svelte'
  import PartyPanel from './PartyPanel.svelte'
  import FriendRequestToast from './FriendRequestToast.svelte'
  import FriendPanel from './FriendPanel.svelte'
  import EmotePanel from './EmotePanel.svelte'
  import InstrumentPanel from './InstrumentPanel.svelte'
  import NpcContextMenu from './NpcContextMenu.svelte'
  import DragGhost from './DragGhost.svelte'
  import LoadingDialog from './LoadingDialog.svelte'
  import RespawnDialog from './RespawnDialog.svelte'
  import TipHatDialog from './TipHatDialog.svelte'
  import CapeDyeDialog from './CapeDyeDialog.svelte'
  import LandClaimDialog from './LandClaimDialog.svelte'
  import { landClaimDialog } from '../stores/landClaimStore'
  import { capeDyeDialog } from '../stores/capeDyeStore'
  import CapeTextureDialog from './CapeTextureDialog.svelte'
  import { capeTextureDialog } from '../stores/capeTextureStore'
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
  import { minimapEnabled } from '../stores/minimapStore'
  import { bubbleLayer } from '../stores/bubbleLayerStore'
  import { friendPanelVisible } from '../stores/friendStore'
  import { emotePanelVisible } from '../stores/emoteStore'
  import { mountOverlay } from '../stores/overlayStack'
  import { networkManager, type AccountCharacter } from '../network/socket'
  import { tipHatDialog } from '../stores/tipHatStore'

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

  let clusterWidth = $state(0)

  const playerLevel = $derived(
    currentPlayerLevel ?? selectedCharacter?.level ?? 1
  )
  const playerXp = $derived(currentPlayerTotalXp ?? selectedCharacter?.xp ?? 0)
  const playerHp = $derived(currentPlayerHp ?? selectedCharacter?.max_hp ?? 0)
  const playerMaxHp = $derived(
    currentPlayerMaxHp ?? selectedCharacter?.max_hp ?? 0
  )

  let socialMenuOpen = $state(false)

  // Every way the flyout closes, mounted only while it is open: Escape
  // through the overlay stack (like the chat channel menu), a pointer down
  // outside it, and focusing the chat input — Enter hides Escape from the
  // stack, so an open flyout would eat the next unfocused Escape.
  $effect(() => {
    if (!socialMenuOpen) return
    const close = () => (socialMenuOpen = false)

    const onPointerDown = (event: PointerEvent) => {
      if (!(event.target as Element | null)?.closest('.social-wrap')) close()
    }
    const onFocusIn = (event: FocusEvent) => {
      const tag = (event.target as Element | null)?.tagName?.toLowerCase()
      if (tag === 'input' || tag === 'textarea') close()
    }

    window.addEventListener('pointerdown', onPointerDown)
    window.addEventListener('focusin', onFocusIn)
    const unmount = mountOverlay('socialMenu', close)

    return () => {
      window.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('focusin', onFocusIn)
      unmount()
    }
  })

  function toggleFromSocialMenu(panel: Writable<boolean>) {
    socialMenuOpen = false
    panel.update((v) => !v)
  }
</script>

<FencePlacementBar />

<div class="game-hud" style:--cluster-width="{clusterWidth}px">
  <ServerNotice />
  <div class="top-left-hud">
    {#if selectedCharacter && !$mapEditorMode}
      <div class="status-row">
        <LevelBadge
          level={playerLevel}
          xp={playerXp}
          hp={playerHp}
          maxHp={playerMaxHp}
          str={selectedCharacter.attributes.str}
        />
        <HungerIndicator />
      </div>
    {/if}
    <FPSCounter />
  </div>
  <WavePhaseDebug />
  <GameTimeWidget />
  {#if $minimapEnabled && !$mapEditorMode}
    <Minimap />
  {/if}
  <div
    class="bubble-layer"
    bind:this={() => $bubbleLayer, (node) => bubbleLayer.set(node)}
  ></div>
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
      level={playerLevel}
      currentXp={playerXp}
      currentHp={playerHp}
      maxHp={playerMaxHp}
      attributes={selectedCharacter.attributes}
      onClose={() => characterPanelVisible.set(false)}
    />
    <InventoryPanel
      visible={$inventoryVisible}
      str={selectedCharacter.attributes.str}
      onClose={() => inventoryVisible.set(false)}
    />
    <TradeWindow />
    <PlayerTradeWindow />
    <TradeOfferToast />
    <PartyInviteToast />
    <PlayerTradeRequestToast />
    <PartySummonToast />
    <PartyPanel />
    <FriendRequestToast />
    <!-- Always mounted: it drives the presence poll, whose answers feed the
         online notice whether or not the list is on screen. -->
    <FriendPanel />
    <EmotePanel />
    <InstrumentPanel />
    <NpcContextMenu />
    <FishingPrompt />
  {/if}

  {#if !$mapEditorMode}
    <ChatPanel />
  {/if}
  <div class="action-cluster" bind:clientWidth={clusterWidth}>
    {#if selectedCharacter && !$mapEditorMode}
      <div class="quickslot-stack">
        <QuickslotBar characterId={selectedCharacter.id} />
      </div>
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
      <div class="social-wrap">
        {#if socialMenuOpen}
          <div class="social-menu">
            <button
              class="social-item"
              onclick={() => toggleFromSocialMenu(friendPanelVisible)}
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="640"
                height="512"
                viewBox="0 0 640 512"
                ><path
                  fill="currentColor"
                  d="M144 0a80 80 0 1 1 0 160A80 80 0 1 1 144 0M512 0a80 80 0 1 1 0 160a80 80 0 1 1 0-160M0 298.7C0 239.8 47.8 192 106.7 192h42.7c15.9 0 31 3.5 44.6 9.7c-1.3 7.2-1.9 14.7-1.9 22.3c0 38.2 16.8 72.5 43.3 96c-.2 0-.4 0-.7 0H21.3C9.6 320 0 310.4 0 298.7zM405.3 320c-.2 0-.4 0-.7 0c26.6-23.5 43.3-57.8 43.3-96c0-7.6-.7-15-1.9-22.3c13.6-6.3 28.7-9.7 44.6-9.7h42.7C592.2 192 640 239.8 640 298.7c0 11.8-9.6 21.3-21.3 21.3H405.3zM224 224a96 96 0 1 1 192 0a96 96 0 1 1-192 0M128 485.3C128 411.7 187.7 352 261.3 352H378.7C452.3 352 512 411.7 512 485.3c0 14.7-11.9 26.7-26.7 26.7H154.7c-14.7 0-26.7-11.9-26.7-26.7z"
                /></svg
              >
              <span class="social-label">Friends</span>
              <span class="key-hint">F</span>
            </button>
            <button
              class="social-item"
              onclick={() => toggleFromSocialMenu(emotePanelVisible)}
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="512"
                height="512"
                viewBox="0 0 512 512"
                ><path
                  fill="currentColor"
                  d="M464 256A208 208 0 1 1 48 256a208 208 0 1 1 416 0zM256 0a256 256 0 1 0 0 512A256 256 0 1 0 256 0zM164.1 325.5C182 346.2 212.6 368 256 368s74-21.8 91.9-42.5c5.8-6.7 15.9-7.4 22.6-1.6s7.4 15.9 1.6 22.6C349.8 372.1 311.1 400 256 400s-93.8-27.9-116.1-53.5c-5.8-6.7-5.1-16.8 1.6-22.6s16.8-5.1 22.6 1.6zM144.4 208a32 32 0 1 1 64 0a32 32 0 1 1 -64 0zm192-32a32 32 0 1 1 0 64a32 32 0 1 1 0-64z"
                /></svg
              >
              <span class="social-label">Emotes</span>
              <span class="key-hint">G</span>
            </button>
          </div>
        {/if}
        <button
          class="corner-btn"
          class:menu-open={socialMenuOpen}
          onclick={() => (socialMenuOpen = !socialMenuOpen)}
          title="Social"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="640"
            height="512"
            viewBox="0 0 640 512"
            ><path
              fill="currentColor"
              d="m323.4 85.2l-96.8 78.4c-16.1 13-19.2 36.4-7 53.1c12.9 17.8 38 21.3 55.3 7.8l99.3-77.2c7-5.4 17-4.2 22.5 2.8s4.2 17-2.8 22.5L373 188.8L550.2 352H592c26.5 0 48-21.5 48-48V176c0-26.5-21.5-48-48-48h-80.7l-3.9-2.5L434.8 79c-15.3-9.8-33.2-15-51.4-15c-21.8 0-43 7.5-60 21.2m22.8 124.4l-51.7 40.2c-31.5 24.6-77.2 18.2-100.8-14.2c-22.2-30.5-16.6-73.1 12.7-96.8l83.2-67.3c-11.6-4.9-24.1-7.4-36.8-7.4C234 64 215.7 69.6 200 80l-72 48H48c-26.5 0-48 21.5-48 48v128c0 26.5 21.5 48 48 48h108.2l91.4 83.4c19.6 17.9 49.9 16.5 67.8-3.1c5.5-6.1 9.2-13.2 11.1-20.6l17 15.6c19.5 17.9 49.9 16.6 67.8-2.9c4.5-4.9 7.8-10.6 9.9-16.5c19.4 13 45.8 10.3 62.1-7.5c17.9-19.5 16.6-49.9-2.9-67.8z"
            /></svg
          >
        </button>
      </div>
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

{#if $capeDyeDialog}
  <CapeDyeDialog
    onConfirm={(color) => {
      networkManager.sendDyeCape($capeDyeDialog!.instanceId, color)
      capeDyeDialog.set(null)
    }}
    onCancel={() => capeDyeDialog.set(null)}
  />
{/if}

{#if $landClaimDialog}
  <LandClaimDialog />
{/if}

{#if $capeTextureDialog}
  <CapeTextureDialog
    onConfirm={(hash) => {
      networkManager.sendApplyCapeTexture($capeTextureDialog!.instanceId, hash)
      capeTextureDialog.set(null)
    }}
    onCancel={() => capeTextureDialog.set(null)}
  />
{/if}

{#if $tipHatDialog}
  <TipHatDialog
    ownerName={$tipHatDialog.ownerName}
    onConfirm={(copper) => {
      networkManager.sendTipHat($tipHatDialog!.hatId, copper)
      tipHatDialog.set(null)
    }}
    onCancel={() => tipHatDialog.set(null)}
  />
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
    /* Screen-edge gutters and the bottom-row gap, media-queried in one place so
       the action cluster and the chat panel stay aligned. */
    --hud-edge-top: 9px;
    --hud-edge-left: 9px;
    --hud-edge-right: 9px;
    --hud-edge-bottom: 9px;
    --hud-row-gap: 16px;
  }

  /* Allow pointer events on interactive HUD children */
  .game-hud :global(*) {
    pointer-events: auto;
  }

  /* Chat bubbles portal here. No z-index: DOM order puts it over the minimap
     and under every later sibling. Declared after the :global(*) rule above
     so bubbles never swallow clicks. */
  .bubble-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  .bubble-layer,
  .bubble-layer :global(*) {
    pointer-events: none;
  }

  /* Quickslots + menu, bottom-right. Layout-only, so it stays click-through
     (the gaps between panels must pass clicks down to the 3D scene) while its
     real children re-enable pointer events via the :global(*) rule above. */
  .action-cluster {
    position: fixed;
    right: var(--hud-edge-right);
    bottom: var(--hud-edge-bottom);
    z-index: 30;
    display: flex;
    align-items: flex-end;
    gap: var(--hud-row-gap);
    pointer-events: none;
  }

  .top-left-hud {
    position: fixed;
    top: var(--hud-edge-top);
    left: var(--hud-edge-left);
    z-index: 1000;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    pointer-events: none;
  }

  .status-row {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    pointer-events: none;
  }

  .quickslot-stack {
    pointer-events: none;
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

  .corner-btn:hover,
  .corner-btn.menu-open {
    background: rgba(80, 80, 80, 0.95);
    color: #fff;
  }

  .social-wrap {
    position: relative;
    display: flex;
  }

  .social-menu {
    position: absolute;
    bottom: calc(100% + 6px);
    right: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px;
    background: rgba(40, 40, 40, 0.95);
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 8px;
    z-index: 10;
  }

  .social-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    background: none;
    border: none;
    border-radius: 6px;
    color: #ccc;
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
  }

  .social-item:hover {
    background: rgba(255, 255, 255, 0.12);
    color: #fff;
  }

  .social-item svg {
    width: 16px;
    height: 16px;
    flex: none;
  }

  .social-label {
    flex: 1;
    text-align: left;
  }

  .key-hint {
    margin-left: 10px;
    padding: 1px 5px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    border-radius: 4px;
    color: #9a9a9a;
    font-size: 10px;
    line-height: 1.4;
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
    .game-hud {
      --hud-edge-top: max(9px, env(safe-area-inset-top));
      --hud-edge-left: max(9px, env(safe-area-inset-left));
      --hud-edge-right: max(9px, env(safe-area-inset-right));
      --hud-edge-bottom: max(9px, env(safe-area-inset-bottom));
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
    .game-hud {
      --hud-edge-bottom: 2px;
    }
  }
</style>

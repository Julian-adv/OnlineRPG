<script lang="ts">
  import { mountOverlay } from '../stores/overlayStack'
  import { reviveItem } from '../stores/inventoryStore'
  import { networkManager } from '../network/socket'

  interface Props {
    onRespawn: () => void
    onLater: () => void
  }

  let { onRespawn, onLater }: Props = $props()

  function useReviveItem(instanceId: number) {
    networkManager.sendUseItem(instanceId)
    onLater()
  }

  // Escape defers the dialog exactly like the Later button.
  $effect(() => mountOverlay('respawn', onLater))
</script>

<div class="respawn-backdrop">
  <div class="respawn-dialog" role="dialog" aria-modal="true">
    <h2>You Died</h2>
    <p>Would you like to revive?</p>
    <div class="respawn-actions">
      {#if $reviveItem}
        <button
          class="talisman"
          onclick={() => useReviveItem($reviveItem.item.instance_id)}
        >
          Use {$reviveItem.def.name} ({$reviveItem.def.reviveHpPercent}% HP, ×{$reviveItem
            .item.quantity})
        </button>
      {/if}
      <button class="primary" onclick={onRespawn}>Revive in Town</button>
      <button class="secondary" onclick={onLater}>Later</button>
    </div>
  </div>
</div>

<style>
  .respawn-backdrop {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.45);
    z-index: 30;
  }

  .respawn-dialog {
    width: min(380px, calc(100vw - 32px));
    padding: 20px;
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.25);
    background: rgba(16, 16, 16, 0.95);
    color: #f4f4f4;
    text-align: center;
  }

  .respawn-dialog h2 {
    margin: 0 0 8px 0;
    font-size: 22px;
  }

  .respawn-dialog p {
    margin: 0 0 16px 0;
    color: #d4d4d4;
  }

  .respawn-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    justify-content: center;
  }

  .respawn-actions button {
    border: none;
    border-radius: 8px;
    padding: 10px 14px;
    font-size: 14px;
    cursor: pointer;
  }

  .respawn-actions .primary {
    background: #e2b93b;
    color: #1a1a1a;
    font-weight: 700;
  }

  .respawn-actions .secondary {
    background: #3d3d3d;
    color: #f0f0f0;
  }

  .respawn-actions .talisman {
    background: #c9632a;
    color: #fff4e6;
    font-weight: 700;
  }
</style>

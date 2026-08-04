<script lang="ts">
  import { dragMeta, dragPos } from '../stores/dragStore'
</script>

{#if $dragMeta}
  <div class="drag-ghost-wrap" style="left:{$dragPos.x}px;top:{$dragPos.y}px">
    {#if $dragMeta.groupItems && $dragMeta.groupItems.length > 0}
      <div class="drag-ghost-group">
        {#each $dragMeta.groupItems as item, i (i)}
          <div class="drag-ghost-item">
            <img
              class="drag-ghost drag-ghost-small"
              src="/items/{item.icon}"
              alt=""
            />
            <span class="drag-ghost-qty">x{item.quantity}</span>
          </div>
        {/each}
      </div>
    {:else}
      <img class="drag-ghost" src="/items/{$dragMeta.icon}" alt="" />
    {/if}
  </div>
{/if}

<style>
  .drag-ghost-wrap {
    position: fixed;
    transform: translate(-50%, -50%);
    pointer-events: none;
    z-index: 100;
  }

  .drag-ghost {
    display: block;
    width: 48px;
    height: 48px;
    image-rendering: pixelated;
    opacity: 0.85;
    filter: drop-shadow(0 2px 6px rgba(0, 0, 0, 0.6));
  }

  .drag-ghost-group {
    display: flex;
    gap: 4px;
  }

  .drag-ghost-item {
    position: relative;
  }

  .drag-ghost-small {
    width: 36px;
    height: 36px;
  }

  .drag-ghost-qty {
    position: absolute;
    bottom: -4px;
    right: -4px;
    background: rgba(6, 10, 14, 0.9);
    border: 1px solid rgba(255, 255, 255, 0.3);
    border-radius: 999px;
    padding: 0 4px;
    font-family: 'Courier New', monospace;
    font-size: 10px;
    font-weight: 700;
    color: #fff;
  }
</style>

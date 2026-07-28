<script lang="ts">
  import { partyRoster } from '../stores/partyStore'
  import { networkManager } from '../network/socket'

  const roster = $derived($partyRoster)
</script>

{#if roster}
  <div class="party-panel" aria-label="Party">
    <div class="party-header">
      <span class="party-title">Party</span>
      <button
        class="leave-btn"
        title="Leave party"
        onclick={() => networkManager.sendPartyLeave()}
      >
        Leave
      </button>
    </div>
    {#each roster.members as member (member.id)}
      <div class="member">
        {#if member.id === roster.leaderId}
          <span class="leader-star" title="Leader">★</span>
        {/if}
        {member.name}
      </div>
    {/each}
  </div>
{/if}

<style>
  .party-panel {
    position: fixed;
    left: 10px;
    top: 30%;
    z-index: 30;
    min-width: 110px;
    padding: 6px 10px 8px;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 8px;
    background: rgba(6, 10, 14, 0.7);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    pointer-events: auto;
  }

  .party-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 4px;
  }

  .party-title {
    color: #7ec8ff;
    font-weight: 700;
  }

  .leave-btn {
    background: none;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 4px;
    padding: 1px 6px;
    color: #9fb2c3;
    font-family: inherit;
    font-size: 10px;
    cursor: pointer;
  }

  .leave-btn:hover {
    color: #fff;
    border-color: rgba(255, 255, 255, 0.4);
  }

  .member {
    line-height: 1.5;
    white-space: nowrap;
  }

  .leader-star {
    color: #f0c040;
  }
</style>

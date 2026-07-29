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
        <span class="role-slot">
          {#if member.id === roster.leaderId}
            <span class="leader-crown" title="Leader">♛</span>
          {/if}
        </span>
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
    min-width: 132px;
    padding: 8px 12px 10px;
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
    margin-bottom: 6px;
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
    display: flex;
    align-items: center;
    gap: 5px;
    line-height: 1.7;
    white-space: nowrap;
  }

  /* Fixed slot whether or not a crown is shown, so names align. */
  .role-slot {
    width: 12px;
    text-align: center;
    flex: none;
  }

  .leader-crown {
    color: #f0c040;
  }
</style>

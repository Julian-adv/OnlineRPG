<script lang="ts">
  import { SvelteSet } from 'svelte/reactivity'
  import {
    partyRoster,
    PARTY_MAX_MEMBERS,
    classIconPath,
  } from '../stores/partyStore'
  import { gameStore } from '../stores/gameStore'
  import { npcContextMenu, type NpcMenuEntry } from '../stores/npcMenuStore'
  import { networkManager } from '../network/socket'
  import { draggablePanel } from '../actions/draggablePanel'

  const roster = $derived($partyRoster)
  const selfId = $derived($gameStore.currentPlayer?.id)
  const isLeader = $derived(roster !== null && roster.leaderId === selfId)

  let collapsed = $state(false)
  let kickTargetId = $state<number | null>(null)
  // Classes whose icon failed to load; they fall back to the letter badge.
  const failedIcons = new SvelteSet<string>()

  // Derived, so the confirm cannot outlive its subject (kicked, left,
  // disband); the stale id is cleared so a rejoin does not resurrect it.
  const kickTarget = $derived(
    roster?.members.find((m) => m.id === kickTargetId) ?? null
  )
  $effect(() => {
    if (kickTargetId !== null && !kickTarget) kickTargetId = null
  })

  function hpPercent(hp: number, maxHp: number): number {
    if (maxHp === 0) return 0
    return Math.max(0, Math.min(100, (hp / maxHp) * 100))
  }

  function openMemberMenu(e: MouseEvent, member: { id: number; name: string }) {
    const entries: NpcMenuEntry[] = []
    if (member.id === selfId) {
      entries.push({
        label: 'Leave party',
        action: () => networkManager.sendPartyLeave(),
      })
    } else if (isLeader) {
      entries.push(
        {
          label: 'Promote to leader',
          action: () => networkManager.sendPartyPromote(member.id),
        },
        {
          label: 'Kick from party',
          action: () => {
            kickTargetId = member.id
          },
        }
      )
    }
    if (entries.length === 0) return
    e.preventDefault()
    npcContextMenu.set({
      npcName: member.name,
      screenX: e.clientX,
      screenY: e.clientY,
      entries,
    })
  }

  function confirmKick() {
    if (kickTarget) networkManager.sendPartyKick(kickTarget.id)
    kickTargetId = null
  }
</script>

{#if roster}
  <div class="party-panel" aria-label="Party" use:draggablePanel={'party'}>
    <div class="party-header" data-drag-handle>
      <span class="party-title">
        PARTY
        <span class="party-count">
          {roster.members.length}/{PARTY_MAX_MEMBERS}
        </span>
      </span>
      <button
        class="collapse-btn"
        title={collapsed ? 'Expand party panel' : 'Collapse party panel'}
        aria-expanded={!collapsed}
        onclick={() => (collapsed = !collapsed)}
      >
        {collapsed ? '▸' : '▾'}
      </button>
    </div>
    {#if !collapsed}
      {#each roster.members as member (member.id)}
        {@const iconPath = classIconPath(member.class)}
        <div
          class="member"
          class:dead={member.hp === 0}
          role="listitem"
          oncontextmenu={(e) => openMemberMenu(e, member)}
        >
          {#if iconPath && !failedIcons.has(member.class)}
            <img
              class="class-icon"
              src={iconPath}
              alt={member.class}
              width="18"
              height="18"
              onerror={() => failedIcons.add(member.class)}
            />
          {:else}
            <span
              class="class-badge"
              title={member.class}
              aria-label={member.class}
            >
              {member.class.charAt(0).toUpperCase()}
            </span>
          {/if}
          <div class="member-body">
            <div class="member-row">
              <span
                class="member-name"
                class:self={member.id === selfId}
                title={member.name}
              >
                {member.name}
              </span>
              {#if member.id === roster.leaderId}
                <img
                  class="leader-crown"
                  src="/icons/party/leader-crown.svg"
                  alt="Leader"
                  title="Leader"
                  width="16"
                  height="16"
                />
              {/if}
            </div>
            <div class="hp-track">
              <div
                class="hp-fill"
                style="width: {hpPercent(member.hp, member.max_hp)}%"
              ></div>
            </div>
          </div>
        </div>
      {/each}
    {/if}
  </div>

  {#if kickTarget}
    <div class="confirm-backdrop" role="presentation">
      <div class="confirm-box" role="alertdialog" aria-label="Kick from party">
        <p class="confirm-text">Kick {kickTarget.name} from the party?</p>
        <div class="confirm-actions">
          <button class="confirm-kick" onclick={confirmKick}>Kick</button>
          <button class="confirm-cancel" onclick={() => (kickTargetId = null)}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  {/if}
{/if}

<style>
  .party-panel {
    position: fixed;
    left: 10px;
    top: 30%;
    z-index: 30;
    min-width: 172px;
    max-width: 240px;
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
    padding-bottom: 5px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.12);
    margin-bottom: 6px;
  }

  .party-title {
    color: #f0c040;
    font-weight: 700;
    letter-spacing: 0.5px;
  }

  .party-count {
    color: #e6edf3;
  }

  .collapse-btn {
    background: none;
    border: none;
    padding: 0 2px;
    color: #9fb2c3;
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
  }

  .collapse-btn:hover {
    color: #fff;
  }

  .member {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 0;
  }

  .member.dead {
    opacity: 0.45;
  }

  .class-icon {
    flex: none;
  }

  .class-badge {
    flex: none;
    box-sizing: border-box;
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid rgba(243, 246, 248, 0.55);
    border-radius: 4px;
    font-size: 11px;
    font-weight: 700;
    color: #f3f6f8;
  }

  .member-body {
    flex: 1;
    min-width: 0;
  }

  .member-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    line-height: 1.4;
  }

  .member-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .member-name.self {
    color: #f0c040;
  }

  .leader-crown {
    flex: none;
  }

  .hp-track {
    margin-top: 3px;
    height: 5px;
    border-radius: 3px;
    background: rgba(224, 82, 82, 0.18);
    overflow: hidden;
  }

  .hp-fill {
    height: 100%;
    border-radius: 3px;
    background: #e05252;
    transition: width 300ms ease;
  }

  .confirm-backdrop {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.35);
  }

  .confirm-box {
    min-width: 220px;
    padding: 14px 16px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 8px;
    background: rgba(6, 10, 14, 0.95);
    color: #e6edf3;
    font-family: 'Courier New', monospace;
    font-size: 13px;
  }

  .confirm-text {
    margin: 0 0 12px;
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .confirm-actions button {
    padding: 4px 12px;
    border-radius: 4px;
    border: 1px solid rgba(255, 255, 255, 0.18);
    background: none;
    color: inherit;
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
  }

  .confirm-kick {
    border-color: rgba(224, 82, 82, 0.6);
    color: #f08080;
  }

  .confirm-kick:hover {
    background: rgba(224, 82, 82, 0.15);
  }

  .confirm-cancel:hover {
    background: rgba(255, 255, 255, 0.1);
  }
</style>

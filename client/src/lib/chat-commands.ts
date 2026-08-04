import { MathUtils } from 'three'
import { get } from 'svelte/store'
import { gameStore, addChatMessage, isAdminUser } from './stores/gameStore'
import { worldToTileCell } from './components/game-scene/terrain-utils'
import { networkManager } from './network/socket'
import {
  editorHeightManager,
  editorSplatManager,
  editorGrassDataManager,
} from './stores/editorStore'
import {
  riverWireframeVisible,
  shoreWaveDebugVisible,
  passabilityDebugVisible,
} from './stores/debugStore'
import { computeGrassPlacement, regenerateVegMeta } from './utils/grass-data'
import { dungeonManager } from './managers/dungeonManager'
import { chatChannel } from './stores/chatChannelStore'
import { partyRoster } from './stores/partyStore'

/** Every command lives here once: its `/help` line, whether it is admin-only,
 *  and who executes it. */
type Command = {
  desc: string
  admin?: boolean
  /** Client-side handler. Omit it to let the text through to the server. */
  run?: (args: string) => void
}

const COMMANDS: Record<string, Command> = {
  '/help': {
    desc: 'List the available commands',
    run: () => {
      const regular: string[] = []
      const adminOnly: string[] = []
      for (const name of visibleCommandNames()) {
        ;(COMMANDS[name].admin ? adminOnly : regular).push(name)
      }
      const line = (name: string) =>
        addChatMessage({
          text: `${name} — ${COMMANDS[name].desc}`,
          sender: 'system',
        })

      addChatMessage({ text: 'Available commands:', sender: 'system' })
      for (const name of regular) line(name)

      if (adminOnly.length > 0) {
        addChatMessage({ text: 'Admin commands:', sender: 'system' })
        for (const name of adminOnly) line(name)
      }
    },
  },

  '/who': { desc: 'Show how many players are online' },
  '/escape': { desc: 'Return to the starting point when you get stuck' },
  '/w': { desc: 'Send a private message: /w <player> <message>' },
  '/whisper': { desc: 'Send a private message: /whisper <player> <message>' },
  '/r': { desc: 'Reply to the last whisper: /r <message>' },
  '/reply': { desc: 'Reply to the last whisper: /reply <message>' },
  '/block': { desc: 'Block whispers from a player: /block <player>' },
  '/unblock': { desc: 'Unblock a player: /unblock <player>' },
  '/party': { desc: 'Invite a player to your party: /party <player>' },
  '/p': {
    desc: 'Talk to your party and stay in party chat: /p [message]',
    run: (args) => {
      const message = args.trim()
      if (!get(partyRoster)) {
        addChatMessage({
          text: 'Party: you are not in a party.',
          sender: 'system',
        })
        return
      }
      chatChannel.set('party')
      if (message) networkManager.sendPartyChat(message)
    },
  },
  '/s': {
    desc: 'Talk normally and leave party chat: /s [message]',
    run: (args) => {
      const message = args.trim()
      chatChannel.set('say')
      if (message) networkManager.sendChatMessage(message)
    },
  },
  '/give': { desc: 'Give yourself an item: /give <item_id>', admin: true },
  '/notice': {
    desc: 'Set the server banner, or clear it with a bare /notice',
    admin: true,
  },

  '/pos': {
    desc: 'Show your current position',
    run: () => {
      const player = get(gameStore).currentPlayer
      if (player) {
        const pos = player.position
        const { tileX, tileZ, cellX, cellZ } = worldToTileCell(pos.x, pos.z)
        const deg = MathUtils.radToDeg(player.rotation).toFixed(1)
        addChatMessage({
          text: `Position: world(${pos.x.toFixed(1)}, ${pos.y.toFixed(1)}, ${pos.z.toFixed(1)}) tile(${tileX}, ${tileZ}) cell(${cellX}, ${cellZ}) rot(${deg}°)`,
          sender: 'system',
        })
      } else {
        addChatMessage({ text: 'Position: unknown', sender: 'system' })
      }
    },
  },

  '/drop': {
    desc: 'Drop an item in front of you',
    admin: true,
    run: (args) => {
      const player = get(gameStore).currentPlayer
      if (!player) {
        addChatMessage({
          text: 'Drop: player position unknown',
          sender: 'system',
        })
        return
      }

      const itemDefId = args.trim() || 'goblin_sword'
      networkManager.sendDebugDropItem(itemDefId)

      addChatMessage({
        text: `Drop: requested ${itemDefId} near 1m ahead`,
        sender: 'system',
      })
    },
  },

  '/time': {
    desc: 'Jump the game clock to HH[:MM]',
    admin: true,
    run: (args) => {
      const match = args.trim().match(/^(\d{1,2})(?::(\d{1,2}))?$/)
      if (!match) {
        addChatMessage({
          text: 'Usage: /time HH[:MM] — jump the game clock forward to that time (e.g. /time 9:30)',
          sender: 'system',
        })
        return
      }
      const hour = Math.min(parseInt(match[1], 10), 23)
      const minute = Math.min(match[2] ? parseInt(match[2], 10) : 0, 59)
      networkManager.sendDebugSetTime(hour, minute)
      addChatMessage({
        text: `Time: requested jump to ${hour}:${String(minute).padStart(2, '0')}`,
        sender: 'system',
      })
    },
  },

  '/dungeon': {
    desc: 'Enter or adjust the debug dungeon',
    admin: true,
    run: (args) => {
      const player = get(gameStore).currentPlayer
      if (!player) {
        addChatMessage({ text: 'Dungeon: player unknown', sender: 'system' })
        return
      }

      const arg = args.trim()
      if (arg === 'exit') {
        const ent = dungeonManager.entrancePos
        if (ent) {
          networkManager.sendDebugTeleport({ x: ent.x, y: ent.y, z: ent.z })
        }
        dungeonManager.exit()
        addChatMessage({ text: 'Dungeon: exited to surface', sender: 'system' })
        return
      }

      if (arg === 'resetprops' || arg === 'reset-props') {
        const entranceId = dungeonManager.dungeonId
        if (!entranceId) {
          addChatMessage({
            text: 'Dungeon props: no active dungeon',
            sender: 'system',
          })
          return
        }
        networkManager.sendDebugResetDungeonProps(entranceId)
        addChatMessage({
          text: 'Dungeon props: reset requested',
          sender: 'system',
        })
        return
      }

      const requested = Math.max(1, parseInt(arg || '1', 10) || 1)
      if (!dungeonManager.active) {
        // Debug dungeon anchored at the player's current position.
        dungeonManager.enter('debug', {
          x: player.position.x,
          y: player.position.y,
          z: player.position.z,
        })
      }
      const total = dungeonManager.floors.length
      const depth = Math.min(requested, total)
      const layout = dungeonManager.layoutAt(depth)
      if (!layout) {
        addChatMessage({ text: 'Dungeon: layout missing', sender: 'system' })
        return
      }
      const target = dungeonManager.cellCenter(
        depth,
        dungeonManager.shaftExitCell(layout.upShaft)
      )
      dungeonManager.setDepth(depth)
      networkManager.sendDebugTeleport(target)
      addChatMessage({
        text: `Dungeon: depth ${depth}/${total} (rooms=${layout.rooms.length}, spawns=${layout.spawns.length})`,
        sender: 'system',
      })
    },
  },

  '/wireframe': {
    desc: 'Toggle the river wireframe overlay',
    run: () => {
      const next = !get(riverWireframeVisible)
      riverWireframeVisible.set(next)
      addChatMessage({
        text: `River wireframe: ${next ? 'on' : 'off'}`,
        sender: 'system',
      })
    },
  },

  '/shore_wave': {
    desc: 'Toggle the shore-wave debug overlay',
    run: () => {
      const next = !get(shoreWaveDebugVisible)
      shoreWaveDebugVisible.set(next)
      addChatMessage({
        text: `Shore wave debug: ${next ? 'on' : 'off'}`,
        sender: 'system',
      })
    },
  },

  '/passability': {
    desc: 'Toggle the passability overlay',
    run: () => {
      const next = !get(passabilityDebugVisible)
      passabilityDebugVisible.set(next)
      addChatMessage({
        text: `Passability debug: ${next ? 'on' : 'off'} (walls red, furniture orange)`,
        sender: 'system',
      })
    },
  },

  '/regrow': {
    desc: 'Regenerate grass on your current tile',
    admin: true,
    run: () => {
      const player = get(gameStore).currentPlayer
      if (!player) {
        addChatMessage({
          text: 'Regrow: player position unknown',
          sender: 'system',
        })
        return
      }

      const hMgr = get(editorHeightManager)
      const sMgr = get(editorSplatManager)
      const gMgr = get(editorGrassDataManager)
      if (!hMgr || !sMgr || !gMgr) {
        addChatMessage({
          text: 'Regrow: terrain managers not ready',
          sender: 'system',
        })
        return
      }

      const { tileX, tileZ } = worldToTileCell(
        player.position.x,
        player.position.z
      )
      const splatData = sMgr.getSplatData(tileX, tileZ)
      if (!splatData) {
        addChatMessage({
          text: `Regrow: no splatmap for tile(${tileX}, ${tileZ})`,
          sender: 'system',
        })
        return
      }

      addChatMessage({
        text: `Regrow: regenerating grass for tile(${tileX}, ${tileZ})...`,
        sender: 'system',
      })

      regenerateVegMeta(splatData, tileX, tileZ)
      // Refresh GPU texture + mark tile dirty for the debounced save.
      sMgr.setSplatmap(tileX, tileZ, splatData)
      sMgr.markDirty(tileX, tileZ)
      sMgr.saveAllDirty().catch((err) => {
        addChatMessage({
          text: `Regrow: splatmap save failed — ${err}`,
          sender: 'system',
        })
      })

      const data = computeGrassPlacement(tileX, tileZ, splatData, hMgr)
      gMgr.saveGrassData(tileX, tileZ, data).then(
        () => {
          addChatMessage({
            text: `Regrow: done — short=${data.shortCount} tall=${data.tallCount} flower=${data.flowerCount}`,
            sender: 'system',
          })
        },
        (err) => {
          addChatMessage({
            text: `Regrow: grass save failed — ${err}`,
            sender: 'system',
          })
        }
      )
    },
  },
}

/** `/help` first so a bare `/` completes to it. */
const commandNames = [
  '/help',
  ...Object.keys(COMMANDS)
    .filter((n) => n !== '/help')
    .sort(),
]

/** Command names for autocomplete; hides admin commands from non-admins. */
export function visibleCommandNames(): string[] {
  if (get(isAdminUser)) return commandNames
  return commandNames.filter((n) => !COMMANDS[n].admin)
}

export function handleCommand(input: string): boolean {
  const spaceIndex = input.indexOf(' ')
  const name = spaceIndex === -1 ? input : input.slice(0, spaceIndex)
  const args = spaceIndex === -1 ? '' : input.slice(spaceIndex + 1)
  const command = COMMANDS[name]
  if (!command?.run) return false
  if (command.admin && !get(isAdminUser)) {
    addChatMessage({ text: `${name}: admin only`, sender: 'system' })
    return true
  }
  command.run(args)
  return true
}

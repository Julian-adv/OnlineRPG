import { gameStore } from '../stores/gameStore'
import { teleportLoading } from '../stores/debugStore'
import { networkManager } from '../network/socket'
import { wrapWorldX } from '../terrain/world-wrap'

/** Admin teleport: move the local player, tell the server, raise the loading
 *  gate that GameScene lowers once the target terrain is in. Returns the
 *  world-wrapped X actually used. */
export function teleportLocalPlayer(x: number, y: number, z: number): number {
  const wrappedX = wrapWorldX(x)
  gameStore.update((state) => {
    state.currentPlayer?.position.set(wrappedX, y, z)
    return state
  })
  networkManager.sendDebugTeleport({ x: wrappedX, y, z })
  teleportLoading.set(true)
  return wrappedX
}

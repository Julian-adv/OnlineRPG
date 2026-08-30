import { beforeEach, describe, expect, it } from 'vitest'
import { remotePlayerManager } from './remotePlayerManager'
import { SitAnimationName } from '../types/animations'

const ID = 7

function player() {
  return remotePlayerManager.players.get(ID)
}

describe('remote sit', () => {
  beforeEach(() => {
    remotePlayerManager.removePlayer(ID)
    remotePlayerManager.initPlayer(ID, { x: 0, y: 0, z: 0 }, 0)
    remotePlayerManager.handleInteraction(ID, SitAnimationName.SIT, 0.03)
  })

  it('stands up before leaving the seat, keeping the seat offset', () => {
    remotePlayerManager.handleStopInteraction(ID)

    expect(player()?.state).toBe('interact')
    expect(player()?.interactionAnim).toBe(SitAnimationName.SIT_TO_STAND)
    expect(player()?.interactOffsetY).toBe(0.03)
  })

  it('goes idle once the stand-up clip finishes', () => {
    remotePlayerManager.handleStopInteraction(ID)
    remotePlayerManager.handleInteractionFinished(ID)

    expect(player()?.state).toBe('idle')
    expect(player()?.interactionAnim).toBeUndefined()
  })

  it('stays seated when a move arrives', () => {
    remotePlayerManager.setTargetPosition(ID, { x: 3, y: 0, z: 0 }, 0)

    expect(player()?.state).toBe('interact')
    expect(player()?.interactionAnim).toBe(SitAnimationName.SIT)
  })
})

import type { Position } from '../utils/movementUtils'
import { startBattleMusic, stopBattleMusic } from './bgmManager'
import { PLAYER_ATTACK_RANGE_METERS } from '../data/combatTiming'

export interface MonsterInfo {
  state?: string
  isDeadPending?: boolean
}

export type CombatUpdateResult =
  | { action: 'none' }
  | { action: 'idle' }
  | { action: 'chasing'; newTarget?: Position }
  | { action: 'reached_attack_range' }
  | { action: 'attacking'; rotation: number }
  | { action: 'attack_cycle'; monsterId: string; rotation: number }

export class CombatController {
  private _targetMonsterId: string | null = null
  private _attackTimer = 0
  private _attackCounter = 0
  private _lastChaseUpdate = 0

  get targetMonsterId(): string | null {
    return this._targetMonsterId
  }

  get attackCounter(): number {
    return this._attackCounter
  }

  get isInCombat(): boolean {
    return this._targetMonsterId !== null
  }

  beginCombat(monsterId: string, inRange: boolean) {
    const wasInCombat = this._targetMonsterId !== null
    this._targetMonsterId = monsterId
    this._attackTimer = 0
    if (inRange) {
      this._attackCounter = 1
    } else {
      this._attackCounter = 0
      this._lastChaseUpdate = Date.now()
    }
    if (!wasInCombat) startBattleMusic()
  }

  cancelCombat() {
    const wasInCombat = this._targetMonsterId !== null
    this._targetMonsterId = null
    this._attackCounter = 0
    this._attackTimer = 0
    if (wasInCombat) stopBattleMusic()
  }

  private startChase(
    monsterObjPos: Position,
    now = Date.now()
  ): CombatUpdateResult {
    this._lastChaseUpdate = now
    return {
      action: 'chasing',
      newTarget: {
        x: monsterObjPos.x,
        y: monsterObjPos.y,
        z: monsterObjPos.z,
      },
    }
  }

  update(
    deltaTime: number,
    playerPos: Position,
    monsterInfo: MonsterInfo | undefined,
    monsterObjPos: Position | undefined,
    isMoving: boolean,
    cooldownMs: number,
    currentPlayerState: string,
    attackRangeMeters = PLAYER_ATTACK_RANGE_METERS
  ): CombatUpdateResult {
    if (!this._targetMonsterId) return { action: 'none' }

    const isFinishingAttack =
      currentPlayerState === 'attack' && this._attackTimer < cooldownMs

    // Monster data missing or dead (and not finishing attack)
    if (!monsterInfo || (monsterInfo.state === 'dead' && !isFinishingAttack)) {
      this.cancelCombat()
      return { action: 'idle' }
    }

    // Monster mesh not found
    if (!monsterObjPos) {
      this.cancelCombat()
      return { action: 'idle' }
    }

    const dx = monsterObjPos.x - playerPos.x
    const dz = monsterObjPos.z - playerPos.z
    const dist = Math.sqrt(dx * dx + dz * dz)

    if (isMoving) {
      // CHASING phase
      if (dist <= attackRangeMeters) {
        return { action: 'reached_attack_range' }
      }

      // Throttled chase target update
      const now = Date.now()
      if (now - this._lastChaseUpdate >= 1000) {
        return this.startChase(monsterObjPos, now)
      }
      return { action: 'chasing' }
    }

    // COMBAT phase (in range)
    if (dist > attackRangeMeters && !isFinishingAttack) {
      return this.startChase(monsterObjPos)
    }

    // Still in range - rotate and attack
    const rotation = Math.atan2(dx, dz)
    this._attackTimer += deltaTime

    const isMonsterAlive =
      monsterInfo.state !== 'dead' && !monsterInfo.isDeadPending

    if (this._attackTimer >= cooldownMs) {
      // A new attack cycle is about to fire: unlike the break check above this
      // applies even mid-finish, so a target that fled during the swing ends
      // the current swing and re-approaches instead of attacking out of range.
      if (dist > attackRangeMeters) {
        return this.startChase(monsterObjPos)
      }

      if (isMonsterAlive) {
        this._attackTimer = 0
        this._attackCounter++
        return {
          action: 'attack_cycle',
          monsterId: this._targetMonsterId,
          rotation,
        }
      } else {
        this.cancelCombat()
        return { action: 'idle' }
      }
    }

    return { action: 'attacking', rotation }
  }
}

export const combatController = new CombatController()

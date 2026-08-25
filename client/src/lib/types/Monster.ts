export interface MonsterData {
  id: string
  type: string
  position: { x: number; y: number; z: number }
  rotation: number
  state: 'idle' | 'walk' | 'run' | 'attack' | 'hit' | 'dead'
  ownerId?: number
  targetPosition?: { x: number; y: number; z: number }
  syncCorrection?: { x: number; z: number } // server offset still to absorb
  targetPlayerId?: number // Who the monster is attacking
  moveSpeed: number
  stateTimer: number
  attackCounter?: number
  hitCounter?: number
  deadPendingTimer?: number
  lastAttackStartedAt?: number
  impactDelay?: number // Delay until hit state starts
  isLastHitSuccess?: boolean // Whether the last attack was a hit
  isDeadPending?: boolean // Death packet received, waiting for impact/hit visuals
  droppedWeaponItemDefId?: string
  lastDamageInfo?: {
    damage: number
    hit: boolean
    trigger: number
  }
  pendingDamage?: number // Temporary storage for impact sync
  pendingSwordHitSoundUrl?: string
  // Damage number scheduled from the attack start. Captures damage/hit at
  // schedule time to survive a follow-up attack overwriting pendingDamage.
  pendingDamageText?: { delay: number; damage: number; hit: boolean }
  health: number
  maxHealth: number
  spawnPosition?: { x: number; y: number; z: number }
  currentFloor?: number
  /** Wire floor_level: 0 = overworld, 1..3 housing, negative = dungeon depth. */
  floorLevel?: number
}

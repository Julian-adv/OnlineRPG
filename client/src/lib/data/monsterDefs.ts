import monstersJson from '../../../../data/monsters.json'

export interface MonsterDefinition {
  id: string
  name: string
  model: string
  health?: number
  level: number
  guard: number
  attackBonus?: number
  walkSpeed: number
  runSpeed: number
  attackRange: number
  chaseRange: number
  attackCooldown: number
  attackImpactDelay: number
  attackDamageTextDelay: number
  behavior: string
  damageRoll?: string
  animIdle: string
  animWalk: string
  animRun: string
  /** One clip, or several `|`-separated to pick from at random per swing. */
  animAttack: string
  animAttackIdle?: string
  /** Empty for monsters on the shared packs — those have no hit reaction. */
  animHit?: string
  animDie: string
  animDead: string
  /**
   * Extra world metres (independent of `scale`) applied after the corpse is
   * auto-grounded on its lowest vertex; negative sinks it. Only needed when a
   * dangling appendage pegs the offset and leaves the body hovering, as the
   * kobold's tail does.
   */
  corpseGroundOffset?: number
  material?: string
  /**
   * When true (default), a killing blow plays the hit reaction before the death
   * clip. Set false for monsters whose hit clip looks awkward as a death lead-in
   * (e.g. scp939's long additive stagger).
   */
  deathPlaysHit?: boolean
  /**
   * Cross-fade seconds into the death clip (default 0.2). Raise for monsters
   * whose death pose is a static clip the body should settle into slowly
   * (e.g. scp939's 939_Sleeping).
   */
  deathFadeSeconds?: number
  /** Visual scale multiplier (default 1). Purely cosmetic — server-side
   * ranges and collision are unaffected. */
  scale?: number
  /** Dungeon boss: shows a nameplate on the client. */
  boss?: boolean
  /** Optional weapon item id, or legacy model path relative to /models/. */
  weapon?: string
  /** Chance from 0-1 that the weapon is dropped on death. */
  weaponDropChance?: number
  /** Skeleton bone name the weapon is parented to (e.g. 'RightHand'). */
  weaponBone?: string
  /** Play the shared character packs; only for models rigged on the character
   * skeleton, which then ship no clips of their own. */
  sharedAnims?: boolean
}

const monsterDefs = monstersJson as Record<string, MonsterDefinition>

export function getMonsterDef(type: string): MonsterDefinition | undefined {
  return monsterDefs[type]
}

export function splitClipNames(value: string | undefined): string[] {
  return value ? value.split('|').filter(Boolean) : []
}

export function attackClipNames(def: MonsterDefinition | undefined): string[] {
  const names = splitClipNames(def?.animAttack)
  return names.length > 0 ? names : ['Attack']
}

export default monsterDefs

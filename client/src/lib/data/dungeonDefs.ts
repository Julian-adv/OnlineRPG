/**
 * Dungeon entrance registry, embedded at build time from data/dungeons.json
 * (generated from data-src/dungeons.csv). The server embeds the same file,
 * so entrances never travel over the network; the entrance id seeds the
 * deterministic layout generator on both sides.
 */
import dungeonsJson from '../../../../data/dungeons.json'

export interface DungeonEntranceDef {
  id: string
  name: string
  x: number
  y: number
  z: number
  /** Semicolon-separated item ids the final-floor chest always yields; server-side only. */
  chestDrops?: string
  /** Fixed floor count override; blank = seed-derived 5..=20. INERT here: the
   *  shared generator reads data-src/dungeons.csv at compile time — editing
   *  this JSON field changes nothing. Listed only to document the schema. */
  floors?: number
  /** Final-floor boss monster type; blank = goblin_boss. INERT here, like `floors`. */
  boss?: string
  /** Random chest-loot ceiling (items.csv `chestTier`); blank = 1. Server-side only. */
  chestTier?: number
}

export const DUNGEON_ENTRANCES: DungeonEntranceDef[] = Object.values(
  dungeonsJson as Record<string, DungeonEntranceDef>
)

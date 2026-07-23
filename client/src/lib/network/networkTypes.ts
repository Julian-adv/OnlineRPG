import type { MonsterData } from '../types/Monster'
import type { WallDirection } from '../utils/house-geometry'

export type Position = {
  x: number
  y: number
  z: number
}

export type CharacterClass =
  | 'knight'
  | 'barbarian'
  | 'rogue'
  | 'caveman'
  | 'valkyrie'
  | 'ranger'
  | 'priest'
  | 'merchant'
  | 'guard'

export type Gender = 'male' | 'female'

export type ServerPlayer = {
  id: number
  name: string
  position: Position
  rotation: number
  level: number
  health: number
  max_health: number
  class: CharacterClass
  gender: Gender
  is_official_npc: boolean
  torch_on: boolean
  floor_level: number
  object_type?: string
}

export type ServerMonster = {
  id: string
  monster_type: string
  position: Position
  rotation: number
  state: MonsterData['state']
  owner_id?: number
  health: number
  max_health: number
  /** 0 = overworld, 1..3 housing floors, negative = dungeon depth. Always
   *  sent by the server (shared Monster::floor_level). */
  floor_level: number
  /** Proactive (선공형): attacks on sight rather than only retaliating.
   *  Drives behavior-tree selection for monsters we own. */
  aggressive?: boolean
}

export type AccountCharacter = {
  id: number
  name: string
  created_at: number
  level: number
  xp: number
  max_hp: number
  attributes: CharacterAttributes
  class: CharacterClass
  gender: Gender
}

export type CharacterAttributes = {
  str: number
  dex: number
  con: number
  int: number
  wis: number
  cha: number
  guard: number
}

export type CharacterRollResult = {
  attributes: CharacterAttributes
  maxHp: number
}

export type RollCharacterStatsResult =
  | {
      ok: true
      attributes: CharacterAttributes
      maxHp: number
    }
  | {
      ok: false
      message: string
    }

// Serde externally tagged enum shapes
export type ClientMessage =
  | {
      ClientInfo: {
        protocol_version: number
        client_kind: string
        client_version: string
      }
    }
  | {
      Authenticate: {
        google_id_token: string
      }
    }
  | {
      CreateCharacter: {
        character_name: string
        character_class: CharacterClass
        gender: Gender
      }
    }
  | { DeleteCharacter: { character_id: number } }
  | { RollCharacterStats: { character_class: CharacterClass; gender: Gender } }
  | { EnterGame: { character_id: number } }
  | {
      PlayerMove: {
        position: Position
        rotation: number
        floor_level: number
        append: boolean
      }
    }
  | { PlayerFloorChanged: { floor_level: number } }
  | { ChatMessage: { message: string } }
  | {
      RequestSpawnMonster: {
        monster_type: string
        position: Position
        rotation: number
      }
    }
  | {
      MonsterMove: {
        monster_id: string
        position: Position
        rotation: number
        state: MonsterData['state']
        target_position: Position
      }
    }
  | { PlayerAttack: { monster_id: string } }
  | { MonsterAttack: { monster_id: string; target_player_id: number } }
  | 'RequestRespawn'
  | { FishingCast: { position: Position } }
  | { FishingRespond: { action: FishingAction } }
  | 'FishingStop'
  | { OpenDungeonChest: { entrance_id: string } }
  | {
      BreakDungeonProp: { entrance_id: string; depth: number; prop_id: number }
    }
  | {
      OpenDungeonProp: { entrance_id: string; depth: number; prop_id: number }
    }
  | {
      ToggleDungeonDoor: {
        entrance_id: string
        depth: number
        door_id: number
      }
    }
  | { RequestDungeonDoors: { entrance_id: string } }
  | { DebugTeleport: { position: Position } }
  | { DebugDropItem: { item_def_id: string } }
  | { DebugSetTime: { hour: number; minute: number } }
  | { DebugResetDungeonProps: { entrance_id: string } }
  | { TorchToggle: { enabled: boolean } }
  | {
      ToggleDoor: {
        house_id: string
        room_index: number
        wall_dir: WallDirection
        segment_index: number
      }
    }
  | { InteractObject: { object_type: string; object_id: number } }
  | 'StopInteraction'
  | 'Heartbeat'
  | { EquipItem: { instance_id: number } }
  | { UnequipItem: { slot: EquipSlot } }
  | { DropItem: { instance_id: number } }
  | 'PickupStarted'
  | { PickupItem: { instance_id: number } }
  | { UseItem: { instance_id: number } }
  | { OpenShop: { merchant_player_id: number } }
  | { CloseShop: { merchant_player_id: number } }
  | { BuyItem: { merchant_player_id: number; item_def_id: string } }
  | { SellItem: { merchant_player_id: number; instance_id: number } }
  | { BuybackItem: { merchant_player_id: number; entry_id: number } }

export type EquipSlot =
  | 'head'
  | 'main_hand'
  | 'off_hand'
  | 'chest'
  | 'ear'
  | 'neck'
  | 'belt'
  | 'pants'
  | 'boots'
  | 'ring'
  | 'ring_left'

export type ItemInstance = {
  instance_id: number
  item_def_id: string
  quantity: number
  /** Weapon enchantment level (+N to attack and damage rolls). */
  enchant: number
}

export type PlayerInventory = {
  bag: ItemInstance[]
  equipped: Partial<Record<EquipSlot, ItemInstance>>
}

/** Trained-skill ids (shared `SkillId` wire strings). */
export type SkillId = 'fishing'

/** Shared `FishingAction` wire strings (`ClientMessage::FishingRespond`). */
export type FishingAction = 'hook'

/** Shared `FishingOutcome` (`ServerMessage::FishingEnded`), in its
 *  externally-tagged serde shape. */
export type FishingOutcome =
  | {
      Caught: { item_def_id: string; size_cm: number; trophy: boolean }
    }
  | 'Escaped'
  | 'Aborted'

export type SkillProgress = {
  level: number
  xp: number
}

/** Per-character trained skills (`ServerMessage::SkillsUpdate` payload).
 *  Absent key = never trained (level 0). */
export type Skills = {
  map: Partial<Record<SkillId, SkillProgress>>
}

export type ServerGroundItem = {
  instance_id: number
  item_def_id: string
  position: Position
  floor_level: number
  /** Carries a dropped weapon's enchantment across the drop/pickup cycle. */
  enchant: number
}

export type AuthSuccessPayload = {
  accountName: string
  characters: AccountCharacter[]
}

/** Where the server actually has the local player after refusing a step. */
export type PositionCorrection = {
  x: number
  y: number
  z: number
  rotation: number
  floorLevel: number
}

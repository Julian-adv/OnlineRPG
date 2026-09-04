import type { MonsterData } from '../types/Monster'
import type { WallDirection } from '../utils/house-geometry'
import type { ClientEnvReport } from '../utils/clientEnvReport'

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
  | 'bard'
  | 'merchant'
  | 'guard'
  | 'maid'

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
  /** Placement id of the occupied chair/bed (v47); absent for emotes. */
  object_id?: number | null
  main_hand?: string | null
  back?: string | null
  back_color?: string | null
  back_texture?: string | null
  /** Carrying the `wet` soaking — drives the footprint trail (doc/DEBUFF.md). */
  wet?: boolean
  /** Shown title id (doc/TITLES.md). */
  title?: string | null
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
  equipment?: VisibleEquipment
  titles?: string[]
  active_title?: string | null
}

/** Equipped item def ids the character-select preview renders. */
export type VisibleEquipment = {
  main_hand?: string | null
  off_hand?: string | null
  back?: string | null
  /** Dye on the worn cape, so a dyed cape looks dyed at character select. */
  back_color?: string | null
  /** Content hash of the print on it, for the same reason. */
  back_texture?: string | null
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
  | { RenameCharacter: { character_id: number; new_name: string } }
  | { RollCharacterStats: { character_class: CharacterClass; gender: Gender } }
  | { EnterGame: { character_id: number } }
  | 'WorldReady'
  | {
      PlayerMove: {
        position: Position
        rotation: number
        floor_level: number
        append: boolean
        sprinting: boolean
      }
    }
  | { PlayerFloorChanged: { floor_level: number } }
  | { ChatMessage: { message: string } }
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
  | 'StartInstrument'
  | { InstrumentNotes: { events: InstrumentNoteWireEvent[] } }
  | { PlayerTradeRequest: { target_name: string } }
  | { PlayerTradeAtStall: { stall_id: number } }
  | { PlayerTradeRespond: { requester_id: number; accept: boolean } }
  | {
      PlayerTradeSetOffer: {
        items: { instance_id: number; quantity: number }[]
        copper: number
      }
    }
  | { PlayerTradeLock: { revision: number } }
  | 'PlayerTradeUnlock'
  | { PlayerTradeConfirm: { revision: number } }
  | 'PlayerTradeCancel'
  | { PartyInvite: { target_name: string } }
  | { PartyRespond: { inviter_id: number; accept: boolean } }
  | { PartySummonRespond: { caster_id: number; accept: boolean } }
  | 'PartyLeave'
  | { PartyKick: { target_id: number } }
  | { PartyPromote: { target_id: number } }
  | { PartyChat: { message: string } }
  | 'RequestPartyPositions'
  | { FriendRespond: { requester_id: number; accept: boolean } }
  | { FriendRemove: { name: string } }
  | 'RequestFriendsOnline'
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
  | { SetActiveTitle: { title: string | null } }
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
  | { SelectAmmo: { item_def_id: string | null } }
  | { UnequipItem: { slot: EquipSlot } }
  | { DropItem: { instance_id: number } }
  | { DropItems: { items: BagLineItem[] } }
  | 'PickupStarted'
  | { PickupItem: { instance_id: number } }
  | { UseItem: { instance_id: number } }
  | { DyeCape: { instance_id: number; color: string } }
  | { ApplyCapeTexture: { instance_id: number; texture: string } }
  | { ReportCapeTexture: { player_id: number } }
  | { TipHat: { hat_id: number; amount: number } }
  | { EatMeal: { meal_id: number } }
  | { OpenShop: { merchant_player_id: number } }
  | { CloseShop: { merchant_player_id: number } }
  | { DeclineTrade: { merchant_player_id: number } }
  | { BuyItem: { merchant_player_id: number; item_def_id: string } }
  | { SellItem: { merchant_player_id: number; instance_id: number } }
  | { BuybackItem: { merchant_player_id: number; entry_id: number } }
  | { BuyItems: { merchant_player_id: number; items: TradeLineItem[] } }
  | { SellItems: { merchant_player_id: number; items: BagLineItem[] } }
  | { BuybackItems: { merchant_player_id: number; entry_ids: number[] } }
  | { EnvReport: ClientEnvReport }

export type InstrumentNoteWireEvent = {
  note: number
  offset_ms: number
}

/** One line of a batched `BuyItems` request: buy `qty` units of one item def. */
export type TradeLineItem = {
  item_def_id: string
  qty: number
}

/** One line of a batched `SellItems`/`DropItems` request: act on `qty` units
 *  of one bag stack. */
export type BagLineItem = {
  instance_id: number
  qty: number
}

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
  | 'hands'
  | 'back'
  | 'shirt'

export type ItemInstance = {
  instance_id: number
  item_def_id: string
  quantity: number
  /** Weapon enchantment level (+N to attack and damage rolls). */
  enchant: number
  /** Dye on this cape (`#rrggbb`), overriding the def's `capeColor`. */
  cape_color?: string | null
  /** Content hash of the print on this cape. */
  cape_texture?: string | null
}

export type PlayerInventory = {
  bag: ItemInstance[]
  equipped: Partial<Record<EquipSlot, ItemInstance>>
  /** Which bag stack the next shot draws from. Ammunition is stackable and
   *  so cannot sit in an equip slot; this names the pile instead. */
  active_ammo?: string | null
}

/** Trained-skill ids (shared `SkillId` wire strings). */
export type SkillId = 'fishing'

/** Shared `FishingAction` wire strings (`ClientMessage::FishingRespond`).
 *  `hook` answers a bite; the rest are held stances during the fight. */
export type FishingAction = 'hook' | 'reel' | 'giveline' | 'hold'

/** Shared `FishState` wire strings (`ServerMessage::FishingFight`). */
export type FishState = 'running' | 'resting' | 'exhausted'

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
  /** Units in the pile; only stackable defs ever exceed 1. */
  quantity: number
  /** Carries a dropped weapon's enchantment across the drop/pickup cycle. */
  enchant: number
  /** The player who put it there, if one did; null for loot and world drops. */
  dropped_by: number | null
}

export type ServerCampfire = {
  id: number
  position: Position
  floor_level: number
}

export type ServerStall = {
  id: number
  owner: number
  position: Position
  rotation: number
  floor_level: number
}

export type ServerTipHat = {
  id: number
  owner: number
  owner_name: string
  position: Position
  rotation: number
  floor_level: number
}

/** A dish the inn maid set on a table in front of a seated guest. */
export type ServerMeal = {
  id: number
  item_def_id: string
  chair_object_id: number
  for_player: number
  position: Position
  rotation: number
  floor_level: number
  eaten: boolean
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
}

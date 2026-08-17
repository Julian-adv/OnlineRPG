//! Character traits: gender, class, rolled attributes, and the saved-character
//! record returned to the client. `CharacterClass` carries enough behaviour
//! (hit-die size, gendered stat adjustments, string round-trip) that it
//! pulls in `Gender` here too, and `Character` lives next to its
//! `CharacterAttributes` so the rolled-stats payload type isn't separated
//! from the persistent record it ends up inside.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Gender {
    #[default]
    #[serde(rename = "male")]
    Male,
    #[serde(rename = "female")]
    Female,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CharacterClass {
    #[serde(rename = "knight")]
    Knight,
    #[serde(rename = "barbarian")]
    Barbarian,
    #[serde(rename = "caveman")]
    Caveman,
    #[serde(rename = "valkyrie")]
    Valkyrie,
    #[serde(rename = "ranger")]
    Ranger,
    #[serde(rename = "samurai")]
    Samurai,
    #[serde(rename = "monk")]
    Monk,
    #[serde(rename = "priest")]
    Priest,
    #[serde(rename = "archaeologist")]
    Archaeologist,
    #[serde(rename = "healer")]
    Healer,
    #[serde(rename = "rogue")]
    Rogue,
    #[serde(rename = "wizard")]
    Wizard,
    #[serde(rename = "tourist")]
    Tourist,
    #[serde(rename = "bard")]
    Bard,
    #[serde(rename = "merchant")]
    Merchant,
    #[serde(rename = "guard")]
    Guard,
}

impl CharacterClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            CharacterClass::Knight => "knight",
            CharacterClass::Barbarian => "barbarian",
            CharacterClass::Caveman => "caveman",
            CharacterClass::Valkyrie => "valkyrie",
            CharacterClass::Ranger => "ranger",
            CharacterClass::Samurai => "samurai",
            CharacterClass::Monk => "monk",
            CharacterClass::Priest => "priest",
            CharacterClass::Archaeologist => "archaeologist",
            CharacterClass::Healer => "healer",
            CharacterClass::Rogue => "rogue",
            CharacterClass::Wizard => "wizard",
            CharacterClass::Tourist => "tourist",
            CharacterClass::Bard => "bard",
            CharacterClass::Merchant => "merchant",
            CharacterClass::Guard => "guard",
        }
    }

    /// Whether a player may create a character of this class. Merchant and
    /// Guard belong to operator-run NPCs: Merchant's CHA +3 widens the
    /// haggling band and Guard is a d10 hit die with STR/CON +2, so both are
    /// balance decisions rather than security ones — the same rule applies to
    /// human and agent players alike (`doc/REMOTE_AGENT_CLIENT.md`).
    pub fn is_player_selectable(&self) -> bool {
        !matches!(self, CharacterClass::Merchant | CharacterClass::Guard)
    }

    pub fn hit_die(&self) -> u8 {
        match self {
            CharacterClass::Knight
            | CharacterClass::Barbarian
            | CharacterClass::Caveman
            | CharacterClass::Valkyrie => 10,
            CharacterClass::Ranger
            | CharacterClass::Samurai
            | CharacterClass::Monk
            | CharacterClass::Priest => 8,
            CharacterClass::Archaeologist
            | CharacterClass::Healer
            | CharacterClass::Rogue
            | CharacterClass::Wizard
            | CharacterClass::Bard => 6,
            CharacterClass::Tourist | CharacterClass::Merchant => 4,
            CharacterClass::Guard => 10,
        }
    }

    /// Class-specific stat adjustments [STR, DEX, CON, INT, WIS, CHA].
    /// Applied after 4d6 roll, before 72 rebalancing.
    pub fn stat_adjustments(&self, gender: Gender) -> [i8; 6] {
        match (self, gender) {
            //                                          STR  DEX  CON  INT  WIS  CHA
            (CharacterClass::Barbarian, Gender::Male) => [3, 0, 2, -2, -2, -1],
            (CharacterClass::Barbarian, Gender::Female) => [2, 1, 1, -2, -1, -1],
            (CharacterClass::Caveman, Gender::Male) => [2, 0, 2, -2, 0, -2],
            (CharacterClass::Caveman, Gender::Female) => [1, 1, 1, -2, 1, -2],
            (CharacterClass::Knight, Gender::Male) => [1, -1, 1, -1, 0, 0],
            (CharacterClass::Knight, Gender::Female) => [0, 0, 0, -1, 1, 0],
            (CharacterClass::Valkyrie, _) => [2, 1, 1, -1, -2, -1],
            (CharacterClass::Ranger, _) => [1, 2, 0, -1, 0, -2],
            (CharacterClass::Samurai, _) => [1, 0, 2, -1, 0, -2],
            (CharacterClass::Monk, _) => [-1, 2, 0, -1, 2, -2],
            (CharacterClass::Priest, _) => [-1, -1, 1, -1, 3, -1],
            (CharacterClass::Rogue, _) => [-1, 3, 0, 1, -1, -2],
            (CharacterClass::Archaeologist, _) => [-1, 1, 0, 2, 1, -3],
            (CharacterClass::Healer, _) => [-2, -1, 1, 1, 2, -1],
            (CharacterClass::Wizard, _) => [-2, 0, -1, 3, 2, -2],
            (CharacterClass::Tourist, _) => [-1, 0, -1, 1, -1, 2],
            (CharacterClass::Bard, _) => [-2, 2, -1, 0, -1, 2],
            (CharacterClass::Merchant, _) => [-2, 0, -1, 1, -1, 3],
            (CharacterClass::Guard, _) => [2, 0, 2, -2, -1, -1],
        }
    }
}

impl std::str::FromStr for CharacterClass {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "knight" => Ok(CharacterClass::Knight),
            "barbarian" => Ok(CharacterClass::Barbarian),
            "caveman" => Ok(CharacterClass::Caveman),
            "valkyrie" => Ok(CharacterClass::Valkyrie),
            "ranger" => Ok(CharacterClass::Ranger),
            "samurai" => Ok(CharacterClass::Samurai),
            "monk" => Ok(CharacterClass::Monk),
            "priest" => Ok(CharacterClass::Priest),
            "archaeologist" => Ok(CharacterClass::Archaeologist),
            "healer" => Ok(CharacterClass::Healer),
            "rogue" => Ok(CharacterClass::Rogue),
            "wizard" => Ok(CharacterClass::Wizard),
            "tourist" => Ok(CharacterClass::Tourist),
            "bard" => Ok(CharacterClass::Bard),
            "merchant" => Ok(CharacterClass::Merchant),
            "guard" => Ok(CharacterClass::Guard),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterAttributes {
    pub r#str: u8,
    pub dex: u8,
    pub con: u8,
    pub int: u8,
    pub wis: u8,
    pub cha: u8,
    pub guard: u8,
}

/// The equipped item def ids that show on a character-select preview.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisibleEquipment {
    pub main_hand: Option<String>,
    pub off_hand: Option<String>,
    pub back: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub level: u32,
    pub xp: u64,
    pub max_hp: u32,
    pub attributes: CharacterAttributes,
    pub class: CharacterClass,
    #[serde(default)]
    pub gender: Gender,
    #[serde(default)]
    pub equipment: VisibleEquipment,
}

//! SIDLA wire types: the header taxonomy, the integer-only field enums of
//! the data dictionary, and the `Packet` these assemble into.
//!
//! Control fields deserialize from integers only. A natural-language token
//! where an enum belongs (`"ACT": "attack"`) is a parse error, not a value —
//! that is the first of the three structural barriers against hallucinated
//! output, before the schema matrix in `super::schema` ever runs.

use serde::{Deserialize, Serialize};

/// Communication purpose. Each header fixes which fields may appear, so the
/// header alone decides how a packet is read (see `super::schema`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum Header {
    /// PPLI: an entity reporting its own state and position.
    #[default]
    A,
    /// Track: an observation of another entity, with identification.
    B,
    /// Engage: an action or interaction to carry out.
    C,
    /// Mission: a higher-level objective change.
    D,
}

impl Header {
    #[cfg(test)]
    pub const ALL: [Header; 4] = [Header::A, Header::B, Header::C, Header::D];

    pub fn as_str(self) -> &'static str {
        match self {
            Header::A => "A",
            Header::B => "B",
            Header::C => "C",
            Header::D => "D",
        }
    }
}

/// Declares an enum whose only wire representation is its integer code.
/// `Deserialize` goes through `i64`, so a string, float or bool in that
/// position fails to parse instead of being coerced.
macro_rules! code_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $code:expr),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            #[cfg(test)]
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn code(self) -> i64 {
                match self { $($name::$variant => $code),+ }
            }

            pub fn from_code(code: i64) -> Option<Self> {
                match code {
                    $($code => Some($name::$variant),)+
                    _ => None,
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_i64(self.code())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let code = i64::deserialize(d)?;
                Self::from_code(code).ok_or_else(|| {
                    serde::de::Error::custom(format_args!(
                        "{} has no value {code}",
                        stringify!($name)
                    ))
                })
            }
        }
    };
}

code_enum! {
    /// IFF — identification friend or foe.
    Iff {
        Unknown = 0,
        Friend = 1,
        Hostile = 2,
        Neutral = 3,
    }
}

code_enum! {
    /// STA — an entity's own reported state.
    Sta {
        Idle = 0,
        Moving = 1,
        Engaged = 2,
        Panic = 3,
        Dead = 4,
    }
}

code_enum! {
    /// ACT — the interaction to execute.
    Act {
        None = 0,
        Talk = 1,
        Attack = 2,
        Gift = 3,
        Flee = 4,
    }
}

code_enum! {
    /// OBJ — the standing objective a mission packet installs.
    Obj {
        None = 0,
        Patrol = 1,
        Search = 2,
        Defend = 3,
        Escort = 4,
        Ambush = 5,
        Raid = 6,
        Charge = 7,
        Exterminate = 8,
    }
}

/// Inclusive bounds on REL, the affinity score.
pub const REL_MIN: i32 = -100;
pub const REL_MAX: i32 = 100;

/// A packet field, named independently of the struct so the schema matrix can
/// be indexed by it. Order follows the protocol's field specification table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    Iff,
    Sta,
    Rel,
    Act,
    Sub,
    Tar,
    Obj,
    Loc,
    Hp,
    Msg,
}

impl Field {
    pub const ALL: [Field; 10] = [
        Field::Iff,
        Field::Sta,
        Field::Rel,
        Field::Act,
        Field::Sub,
        Field::Tar,
        Field::Obj,
        Field::Loc,
        Field::Hp,
        Field::Msg,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Field::Iff => "IFF",
            Field::Sta => "STA",
            Field::Rel => "REL",
            Field::Act => "ACT",
            Field::Sub => "SUB",
            Field::Tar => "TAR",
            Field::Obj => "OBJ",
            Field::Loc => "LOC",
            Field::Hp => "HP",
            Field::Msg => "MSG",
        }
    }

    pub fn is_present(self, packet: &Packet) -> bool {
        match self {
            Field::Iff => packet.iff.is_some(),
            Field::Sta => packet.sta.is_some(),
            Field::Rel => packet.rel.is_some(),
            Field::Act => packet.act.is_some(),
            Field::Sub => packet.sub.is_some(),
            Field::Tar => packet.tar.is_some(),
            Field::Obj => packet.obj.is_some(),
            Field::Loc => packet.loc.is_some(),
            Field::Hp => packet.hp.is_some(),
            Field::Msg => packet.msg.is_some(),
        }
    }
}

/// SUB/TAR — an entity identifier. A name is the readable form the spec's
/// worked examples use; a hash is the compact form for a large world. Both
/// are accepted on the wire and compare as distinct identities.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EntityId {
    Name(String),
    Hash(u64),
}

impl EntityId {
    pub fn name(s: impl Into<String>) -> Self {
        EntityId::Name(s.into())
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntityId::Name(n) => f.write_str(n),
            EntityId::Hash(h) => write!(f, "{h}"),
        }
    }
}

/// LOC — a zone identifier or absolute coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Loc {
    Zone(String),
    Coord([f32; 3]),
}

/// A SIDLA data link packet. Every field but the header is optional at the
/// type level; which ones are actually admissible is decided per header by
/// `super::schema::validate`, so an invalid combination is representable but
/// never accepted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Packet {
    #[serde(rename = "H")]
    pub h: Header,
    #[serde(rename = "SUB", default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<EntityId>,
    #[serde(rename = "TAR", default, skip_serializing_if = "Option::is_none")]
    pub tar: Option<EntityId>,
    #[serde(rename = "IFF", default, skip_serializing_if = "Option::is_none")]
    pub iff: Option<Iff>,
    #[serde(rename = "STA", default, skip_serializing_if = "Option::is_none")]
    pub sta: Option<Sta>,
    #[serde(rename = "REL", default, skip_serializing_if = "Option::is_none")]
    pub rel: Option<i32>,
    #[serde(rename = "ACT", default, skip_serializing_if = "Option::is_none")]
    pub act: Option<Act>,
    #[serde(rename = "OBJ", default, skip_serializing_if = "Option::is_none")]
    pub obj: Option<Obj>,
    #[serde(rename = "LOC", default, skip_serializing_if = "Option::is_none")]
    pub loc: Option<Loc>,
    /// Extension: health as a percentage, so a tracked entity's condition is
    /// legible without adding a second numeric channel.
    #[serde(rename = "HP", default, skip_serializing_if = "Option::is_none")]
    pub hp: Option<u8>,
    /// Extension: the spoken line carried by `ACT = Talk`. Dialogue is
    /// payload, not control — no decision is read from this field.
    #[serde(rename = "MSG", default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

impl Packet {
    /// PPLI: `sub` reports being in state `sta` at `loc`.
    pub fn ppli(sub: EntityId, sta: Sta, loc: Loc) -> Self {
        Self {
            h: Header::A,
            sub: Some(sub),
            sta: Some(sta),
            loc: Some(loc),
            ..Default::default()
        }
    }

    /// Track: `sub` observes `tar` and identifies it as `iff`.
    pub fn track(sub: EntityId, tar: EntityId, iff: Iff) -> Self {
        Self {
            h: Header::B,
            sub: Some(sub),
            tar: Some(tar),
            iff: Some(iff),
            ..Default::default()
        }
    }

    /// Engage: `sub` performs `act` on `tar`.
    pub fn engage(sub: EntityId, tar: EntityId, act: Act) -> Self {
        Self {
            h: Header::C,
            sub: Some(sub),
            tar: Some(tar),
            act: Some(act),
            ..Default::default()
        }
    }

    /// Mission: `sub` adopts objective `obj`.
    pub fn mission(sub: EntityId, obj: Obj) -> Self {
        Self {
            h: Header::D,
            sub: Some(sub),
            obj: Some(obj),
            ..Default::default()
        }
    }

    pub fn with_hp(mut self, hp: u8) -> Self {
        self.hp = Some(hp);
        self
    }

    pub fn with_rel(mut self, rel: i32) -> Self {
        self.rel = Some(rel);
        self
    }

    #[cfg(test)]
    pub fn with_msg(mut self, msg: impl Into<String>) -> Self {
        self.msg = Some(msg.into());
        self
    }

    #[cfg(test)]
    pub fn with_tar(mut self, tar: EntityId) -> Self {
        self.tar = Some(tar);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The specification's worked example: one agent sights another it has
    // history with, then acts on it. `Mika`/`Saori` are the specification's
    // illustrative names; in play `SUB` and `TAR` carry engine identifiers.

    #[test]
    fn the_specs_track_example_round_trips() {
        let json = r#"{"H":"B","SUB":"Mika","TAR":"Saori","IFF":2,"REL":-45}"#;
        let packet: Packet = serde_json::from_str(json).unwrap();
        assert_eq!(packet.h, Header::B);
        assert_eq!(packet.sub, Some(EntityId::name("Mika")));
        assert_eq!(packet.tar, Some(EntityId::name("Saori")));
        assert_eq!(packet.iff, Some(Iff::Hostile));
        assert_eq!(packet.rel, Some(-45));
        assert_eq!(serde_json::to_string(&packet).unwrap(), json);
    }

    /// The example's follow-up engagement. Both actions are exercised: the
    /// specification's drafts differ on whether the agent opens with talk or
    /// with an attack, and the wire form has to carry either.
    #[test]
    fn the_specs_engage_example_round_trips_for_either_action() {
        for (json, want) in [
            (r#"{"H":"C","SUB":"Mika","TAR":"Saori","ACT":1}"#, Act::Talk),
            (
                r#"{"H":"C","SUB":"Mika","TAR":"Saori","ACT":2}"#,
                Act::Attack,
            ),
        ] {
            let packet: Packet = serde_json::from_str(json).unwrap();
            assert_eq!(packet.act, Some(want));
            assert_eq!(serde_json::to_string(&packet).unwrap(), json);
        }
    }

    /// A real deployment names entities the way the engine does — a monster by
    /// its instance id, a character by the name the server answers to.
    #[test]
    fn engine_identifiers_are_carried_verbatim() {
        let json = r#"{"H":"C","SUB":"npc_guard_karl","TAR":"monster_slime_00c1","ACT":2}"#;
        let packet: Packet = serde_json::from_str(json).unwrap();
        assert_eq!(packet.sub, Some(EntityId::name("npc_guard_karl")));
        assert_eq!(packet.tar, Some(EntityId::name("monster_slime_00c1")));
        assert_eq!(serde_json::to_string(&packet).unwrap(), json);
    }

    #[test]
    fn absent_fields_are_omitted_not_nulled() {
        let packet = Packet::mission(EntityId::name("Mika"), Obj::Patrol);
        assert_eq!(
            serde_json::to_string(&packet).unwrap(),
            r#"{"H":"D","SUB":"Mika","OBJ":1}"#
        );
    }

    #[test]
    fn a_natural_language_token_cannot_stand_in_for_an_enum() {
        for json in [
            r#"{"H":"C","SUB":"Mika","TAR":"Saori","ACT":"attack"}"#,
            r#"{"H":"B","SUB":"Mika","TAR":"Saori","IFF":"hostile"}"#,
            r#"{"H":"A","SUB":"Mika","STA":"idle","LOC":"Cafe"}"#,
        ] {
            assert!(
                serde_json::from_str::<Packet>(json).is_err(),
                "accepted prose: {json}"
            );
        }
    }

    #[test]
    fn enum_codes_outside_the_dictionary_are_rejected() {
        for json in [
            r#"{"H":"C","SUB":"Mika","TAR":"Saori","ACT":9}"#,
            r#"{"H":"B","SUB":"Mika","TAR":"Saori","IFF":4}"#,
            r#"{"H":"A","SUB":"Mika","STA":5,"LOC":"Cafe"}"#,
            r#"{"H":"D","SUB":"Mika","OBJ":9}"#,
        ] {
            assert!(
                serde_json::from_str::<Packet>(json).is_err(),
                "accepted out-of-dictionary code: {json}"
            );
        }
    }

    #[test]
    fn a_float_is_not_an_enum_code() {
        let json = r#"{"H":"C","SUB":"Mika","TAR":"Saori","ACT":2.5}"#;
        assert!(serde_json::from_str::<Packet>(json).is_err());
    }

    #[test]
    fn an_unrecognised_header_is_rejected() {
        let json = r#"{"H":"E","SUB":"Mika"}"#;
        assert!(serde_json::from_str::<Packet>(json).is_err());
    }

    #[test]
    fn an_invented_field_is_rejected() {
        let json = r#"{"H":"C","SUB":"Mika","TAR":"Saori","ACT":2,"SPELL":"fireball"}"#;
        assert!(serde_json::from_str::<Packet>(json).is_err());
    }

    #[test]
    fn loc_accepts_a_zone_or_coordinates() {
        let zone: Packet =
            serde_json::from_str(r#"{"H":"A","SUB":"Mika","STA":0,"LOC":"Trinity_Cafe"}"#).unwrap();
        assert_eq!(zone.loc, Some(Loc::Zone("Trinity_Cafe".into())));

        let coord: Packet =
            serde_json::from_str(r#"{"H":"A","SUB":"Mika","STA":1,"LOC":[1.5,0.0,-2.5]}"#).unwrap();
        assert_eq!(coord.loc, Some(Loc::Coord([1.5, 0.0, -2.5])));
    }

    #[test]
    fn an_entity_may_be_a_name_or_a_hash() {
        let packet: Packet =
            serde_json::from_str(r#"{"H":"C","SUB":91827,"TAR":"Saori","ACT":2}"#).unwrap();
        assert_eq!(packet.sub, Some(EntityId::Hash(91827)));
        assert_eq!(packet.tar, Some(EntityId::name("Saori")));
    }

    #[test]
    fn every_dictionary_code_maps_both_ways() {
        for v in Iff::ALL {
            assert_eq!(Iff::from_code(v.code()), Some(*v));
        }
        for v in Sta::ALL {
            assert_eq!(Sta::from_code(v.code()), Some(*v));
        }
        for v in Act::ALL {
            assert_eq!(Act::from_code(v.code()), Some(*v));
        }
        for v in Obj::ALL {
            assert_eq!(Obj::from_code(v.code()), Some(*v));
        }
    }

    #[test]
    fn every_header_renders_as_its_own_letter() {
        assert_eq!(Header::ALL.map(Header::as_str), ["A", "B", "C", "D"]);
    }
}

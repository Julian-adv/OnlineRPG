//! Masking control logic: the per-header field matrix and the validator that
//! enforces it.
//!
//! Validation is total — every field of every header has a declared
//! requirement, so there is no combination the matrix leaves undecided. A
//! packet that fails is discarded rather than repaired, and the caller falls
//! back to `super::fsm`.

use super::packet::{Act, Field, Header, Packet, REL_MAX, REL_MIN};

/// Whether a field may, must, or must not appear under a given header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Required,
    Optional,
    Forbidden,
}

/// Why a packet was rejected. Mirrors the exception classes the protocol
/// defines: a missing mandatory field, a field the header forbids, a value
/// outside its declared domain, and a packet that failed to decode at all
/// (which covers prose where an enum belongs).
#[derive(Debug, Clone, PartialEq)]
pub enum Violation {
    MissingRequired {
        header: Header,
        field: Field,
    },
    ForbiddenField {
        header: Header,
        field: Field,
    },
    OutOfRange {
        field: Field,
        value: i64,
        min: i64,
        max: i64,
    },
    /// `MSG` present without `ACT = Talk`: dialogue with nothing to speak on.
    DanglingPayload {
        field: Field,
    },
    /// Failed to decode into a packet — bad JSON, unknown field, or a
    /// non-integer in a control field.
    Malformed(String),
    /// Decoded and valid, but no packet in the frame commands anything.
    NoCommand,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::MissingRequired { header, field } => {
                write!(f, "header {} requires {}", header.as_str(), field.as_str())
            }
            Violation::ForbiddenField { header, field } => {
                write!(f, "header {} forbids {}", header.as_str(), field.as_str())
            }
            Violation::OutOfRange {
                field,
                value,
                min,
                max,
            } => write!(f, "{} = {value} outside {min}..={max}", field.as_str()),
            Violation::DanglingPayload { field } => {
                write!(f, "{} present without ACT = Talk", field.as_str())
            }
            Violation::Malformed(e) => write!(f, "malformed packet: {e}"),
            Violation::NoCommand => f.write_str("frame carries no actionable packet"),
        }
    }
}

/// The requirement for `field` under `header`.
///
/// Core dictionary fields follow the protocol's header specification. The two
/// extension fields are admitted only where they are meaningful: `HP` beside
/// the position it qualifies, `MSG` beside the action it voices.
pub fn rule(header: Header, field: Field) -> Rule {
    use Field::*;
    use Header::*;
    use Rule::*;

    match (header, field) {
        // A (PPLI): who, in what state, where.
        (A, Sub) | (A, Sta) | (A, Loc) => Required,
        (A, Hp) => Optional,
        (A, _) => Forbidden,

        // B (Track): who saw whom, and how it is identified.
        (B, Sub) | (B, Tar) | (B, Iff) => Required,
        (B, Rel) => Optional,
        (B, _) => Forbidden,

        // C (Engage): who does what to whom.
        (C, Sub) | (C, Tar) | (C, Act) => Required,
        (C, Msg) => Optional,
        (C, _) => Forbidden,

        // D (Mission): who adopts which objective, optionally about whom.
        (D, Sub) | (D, Obj) => Required,
        (D, Tar) => Optional,
        (D, _) => Forbidden,
    }
}

/// Validate one packet against its header's field matrix and value domains.
pub fn validate(packet: &Packet) -> Result<(), Violation> {
    for field in Field::ALL {
        let present = field.is_present(packet);
        match rule(packet.h, field) {
            Rule::Required if !present => {
                return Err(Violation::MissingRequired {
                    header: packet.h,
                    field,
                })
            }
            Rule::Forbidden if present => {
                return Err(Violation::ForbiddenField {
                    header: packet.h,
                    field,
                })
            }
            _ => {}
        }
    }

    if let Some(rel) = packet.rel {
        if !(REL_MIN..=REL_MAX).contains(&rel) {
            return Err(Violation::OutOfRange {
                field: Field::Rel,
                value: rel as i64,
                min: REL_MIN as i64,
                max: REL_MAX as i64,
            });
        }
    }

    if let Some(hp) = packet.hp {
        if hp > 100 {
            return Err(Violation::OutOfRange {
                field: Field::Hp,
                value: hp as i64,
                min: 0,
                max: 100,
            });
        }
    }

    if packet.msg.is_some() && packet.act != Some(Act::Talk) {
        return Err(Violation::DanglingPayload { field: Field::Msg });
    }

    Ok(())
}

/// Parse a downlink frame and validate every packet in it. One bad packet
/// condemns the frame: a partially trusted frame would leave the agent acting
/// on half a decision.
pub fn parse_frame(text: &str) -> Result<Vec<Packet>, Violation> {
    let mut packets = Vec::new();
    for line in super::wire::split_frame(text) {
        let packet: Packet = serde_json::from_str(line)
            .map_err(|e| Violation::Malformed(format!("{e} in `{line}`")))?;
        validate(&packet)?;
        packets.push(packet);
    }
    if packets.is_empty() {
        return Err(Violation::Malformed("frame contained no packet".into()));
    }
    Ok(packets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidla::packet::{EntityId, Iff, Loc, Obj, Sta};

    fn mika() -> EntityId {
        EntityId::name("Mika")
    }

    fn saori() -> EntityId {
        EntityId::name("Saori")
    }

    #[test]
    fn the_matrix_decides_every_header_field_pair() {
        for header in Header::ALL {
            for field in Field::ALL {
                let _ = rule(header, field);
            }
        }
    }

    #[test]
    fn each_header_requires_exactly_its_specified_fields() {
        let required = |h: Header| {
            Field::ALL
                .into_iter()
                .filter(|f| rule(h, *f) == Rule::Required)
                .map(Field::as_str)
                .collect::<Vec<_>>()
        };
        assert_eq!(required(Header::A), ["STA", "SUB", "LOC"]);
        assert_eq!(required(Header::B), ["IFF", "SUB", "TAR"]);
        assert_eq!(required(Header::C), ["ACT", "SUB", "TAR"]);
        assert_eq!(required(Header::D), ["SUB", "OBJ"]);
    }

    #[test]
    fn rel_is_optional_on_track_and_forbidden_elsewhere() {
        assert_eq!(rule(Header::B, Field::Rel), Rule::Optional);
        for h in [Header::A, Header::C, Header::D] {
            assert_eq!(rule(h, Field::Rel), Rule::Forbidden);
        }
    }

    #[test]
    fn tar_is_optional_on_mission_and_forbidden_on_ppli() {
        assert_eq!(rule(Header::D, Field::Tar), Rule::Optional);
        assert_eq!(rule(Header::A, Field::Tar), Rule::Forbidden);
    }

    #[test]
    fn well_formed_packets_of_every_header_pass() {
        let packets = [
            Packet::ppli(mika(), Sta::Idle, Loc::Zone("Trinity_Cafe".into())),
            Packet::track(mika(), saori(), Iff::Hostile).with_rel(-45),
            Packet::engage(mika(), saori(), Act::Talk).with_msg("Long time."),
            Packet::mission(mika(), Obj::Patrol).with_tar(saori()),
        ];
        for packet in packets {
            validate(&packet).unwrap_or_else(|e| panic!("rejected valid packet: {e}"));
        }
    }

    #[test]
    fn an_engage_packet_without_act_is_discarded() {
        let mut packet = Packet::engage(mika(), saori(), Act::Attack);
        packet.act = None;
        assert_eq!(
            validate(&packet),
            Err(Violation::MissingRequired {
                header: Header::C,
                field: Field::Act
            })
        );
    }

    #[test]
    fn a_ppli_packet_carrying_a_target_is_treated_as_contaminated() {
        let packet = Packet::ppli(mika(), Sta::Idle, Loc::Zone("Cafe".into())).with_tar(saori());
        assert_eq!(
            validate(&packet),
            Err(Violation::ForbiddenField {
                header: Header::A,
                field: Field::Tar
            })
        );
    }

    #[test]
    fn an_engage_packet_may_not_smuggle_a_position() {
        let mut packet = Packet::engage(mika(), saori(), Act::Attack);
        packet.loc = Some(Loc::Coord([1.0, 0.0, 1.0]));
        assert_eq!(
            validate(&packet),
            Err(Violation::ForbiddenField {
                header: Header::C,
                field: Field::Loc
            })
        );
    }

    #[test]
    fn affinity_beyond_its_bounds_is_rejected() {
        for rel in [-101, 101, i32::MAX] {
            let packet = Packet::track(mika(), saori(), Iff::Neutral).with_rel(rel);
            assert!(matches!(
                validate(&packet),
                Err(Violation::OutOfRange {
                    field: Field::Rel,
                    ..
                })
            ));
        }
        for rel in [-100, 0, 100] {
            let packet = Packet::track(mika(), saori(), Iff::Neutral).with_rel(rel);
            assert_eq!(validate(&packet), Ok(()));
        }
    }

    #[test]
    fn health_above_full_is_rejected() {
        let packet = Packet::ppli(mika(), Sta::Idle, Loc::Zone("Cafe".into())).with_hp(120);
        assert!(matches!(
            validate(&packet),
            Err(Violation::OutOfRange {
                field: Field::Hp,
                ..
            })
        ));
    }

    #[test]
    fn dialogue_without_a_talk_action_is_rejected() {
        let packet = Packet::engage(mika(), saori(), Act::Attack).with_msg("hello");
        assert_eq!(
            validate(&packet),
            Err(Violation::DanglingPayload { field: Field::Msg })
        );
    }

    #[test]
    fn a_frame_is_condemned_by_its_worst_packet() {
        let frame = concat!(
            r#"{"H":"C","SUB":"Mika","TAR":"Saori","ACT":1}"#,
            "\n",
            r#"{"H":"A","SUB":"Mika","STA":0,"LOC":"Cafe","TAR":"Saori"}"#,
        );
        assert!(parse_frame(frame).is_err());
    }

    #[test]
    fn a_valid_multi_packet_frame_parses_in_order() {
        let frame = concat!(
            r#"{"H":"B","SUB":"Mika","TAR":"Saori","IFF":2,"REL":-45}"#,
            "\n",
            r#"{"H":"C","SUB":"Mika","TAR":"Saori","ACT":1}"#,
        );
        let packets = parse_frame(frame).unwrap();
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].h, Header::B);
        assert_eq!(packets[1].act, Some(Act::Talk));
    }

    #[test]
    fn an_empty_frame_is_not_a_valid_decision() {
        assert!(parse_frame("").is_err());
        assert!(parse_frame("   \n \n").is_err());
    }
}

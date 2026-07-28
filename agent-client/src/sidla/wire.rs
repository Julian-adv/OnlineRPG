//! Frame serialisation. Two renderings of the same packets: canonical JSON,
//! and a positional form that drops the key names once both ends agree on the
//! field order a header implies.
//!
//! The protocol is deliberately format-agnostic — only the header/field
//! contract is fixed — so `Wire` is a presentation choice, not a change of
//! meaning. `split_frame` reads back either shape of JSON an LLM emits: one
//! object per line, an array, or objects run together, with or without
//! markdown fences.

use super::packet::{Header, Loc, Packet};

/// How an uplink frame is written out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wire {
    /// One JSON object per line. Self-describing, and what the downlink
    /// always uses.
    #[default]
    Json,
    /// Pipe-separated positional fields. Same information, fewer tokens.
    Compact,
}

pub fn render(packets: &[Packet], wire: Wire) -> String {
    match wire {
        Wire::Json => render_json(packets),
        Wire::Compact => render_compact(packets),
    }
}

pub fn render_json(packets: &[Packet]) -> String {
    packets
        .iter()
        .map(|p| serde_json::to_string(p).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_compact(packets: &[Packet]) -> String {
    packets
        .iter()
        .map(compact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn compact_line(p: &Packet) -> String {
    let sub = p.sub.as_ref().map(|e| e.to_string()).unwrap_or_default();
    let tar = p.tar.as_ref().map(|e| e.to_string()).unwrap_or_default();
    let mut cells: Vec<String> = vec![p.h.as_str().to_string(), sub];

    match p.h {
        Header::A => {
            cells.push(code_cell(p.sta.map(|v| v.code())));
            cells.push(p.loc.as_ref().map(render_loc).unwrap_or_default());
            if let Some(hp) = p.hp {
                cells.push(hp.to_string());
            }
        }
        Header::B => {
            cells.push(tar);
            cells.push(code_cell(p.iff.map(|v| v.code())));
            if let Some(rel) = p.rel {
                cells.push(rel.to_string());
            }
        }
        Header::C => {
            cells.push(tar);
            cells.push(code_cell(p.act.map(|v| v.code())));
            if let Some(ref msg) = p.msg {
                cells.push(msg.replace('|', "/").replace('\n', " "));
            }
        }
        Header::D => {
            cells.push(code_cell(p.obj.map(|v| v.code())));
            if !tar.is_empty() {
                cells.push(tar);
            }
        }
    }
    cells.join("|")
}

fn code_cell(code: Option<i64>) -> String {
    code.map(|c| c.to_string()).unwrap_or_default()
}

fn render_loc(loc: &Loc) -> String {
    match loc {
        Loc::Zone(z) => z.clone(),
        Loc::Coord([x, y, z]) => format!("{x:.1},{y:.1},{z:.1}"),
    }
}

/// Slice out the top-level JSON objects of a frame, ignoring anything between
/// them. Scans with a brace depth counter so braces inside strings and
/// escaped quotes do not split an object.
pub fn split_frame(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' if depth > 0 => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start.take() {
                        spans.push(&text[s..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    spans
}

/// Rough token count, used only to compare two renderings of the same content.
/// Four characters per token is the usual English/JSON approximation.
#[cfg(test)]
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidla::packet::{Act, EntityId, Iff, Obj, Sta};

    fn mika() -> EntityId {
        EntityId::name("Mika")
    }

    fn saori() -> EntityId {
        EntityId::name("Saori")
    }

    #[test]
    fn newline_separated_objects_split() {
        let frame = "{\"H\":\"A\"}\n{\"H\":\"B\"}";
        assert_eq!(split_frame(frame), ["{\"H\":\"A\"}", "{\"H\":\"B\"}"]);
    }

    #[test]
    fn a_markdown_fenced_array_splits() {
        let frame = "```json\n[{\"H\":\"A\"},\n {\"H\":\"C\"}]\n```";
        assert_eq!(split_frame(frame), ["{\"H\":\"A\"}", "{\"H\":\"C\"}"]);
    }

    #[test]
    fn prose_around_the_packets_is_ignored() {
        let frame = "Sure! Here you go:\n{\"H\":\"C\"}\nHope that helps.";
        assert_eq!(split_frame(frame), ["{\"H\":\"C\"}"]);
    }

    #[test]
    fn a_brace_inside_a_string_does_not_split_the_object() {
        let frame = r#"{"H":"C","MSG":"a { brace \" and more }"}"#;
        assert_eq!(split_frame(frame), [frame]);
    }

    #[test]
    fn a_frame_with_no_object_yields_nothing() {
        assert!(split_frame("I would rather not.").is_empty());
    }

    #[test]
    fn compact_drops_the_key_names() {
        let packets = [
            Packet::ppli(mika(), Sta::Moving, Loc::Coord([12.34, 0.0, -4.56])).with_hp(80),
            Packet::track(mika(), saori(), Iff::Hostile).with_rel(-45),
            Packet::engage(mika(), saori(), Act::Talk).with_msg("Long time."),
            Packet::mission(mika(), Obj::Patrol).with_tar(saori()),
        ];
        assert_eq!(
            render_compact(&packets),
            concat!(
                "A|Mika|1|12.3,0.0,-4.6|80\n",
                "B|Mika|Saori|2|-45\n",
                "C|Mika|Saori|1|Long time.\n",
                "D|Mika|1|Saori",
            )
        );
    }

    #[test]
    fn compact_is_cheaper_than_json_for_the_same_packets() {
        let packets: Vec<Packet> = (0..8)
            .map(|i| {
                Packet::ppli(
                    EntityId::name(format!("slime_{i}")),
                    Sta::Idle,
                    Loc::Coord([i as f32, 0.0, -(i as f32)]),
                )
                .with_hp(100)
            })
            .collect();
        let json = estimate_tokens(&render_json(&packets));
        let compact = estimate_tokens(&render_compact(&packets));
        assert!(compact * 2 <= json, "json {json}, compact {compact}");
    }

    /// Where the token saving actually is. The uplink is only modestly cheaper
    /// than `format_world_state`, which is already terse; the reply is where a
    /// prose envelope costs several times what a packet does, because the model
    /// no longer narrates its reasoning to reach a decision.
    #[test]
    fn a_packet_reply_costs_a_fraction_of_a_prose_envelope() {
        let envelope = concat!(
            "{\n",
            "  \"thought\": \"A slime is close and I still have most of my health, so I \
             will engage it before it reaches Saori.\",\n",
            "  \"actions\": [{\"type\": \"attack\", \"monster_id\": \"monster_slime_0003\"}],\n",
            "  \"memory_update\": \"Fought a slime near the cafe while Saori was nearby.\"\n",
            "}",
        );
        let packet = Packet::engage(
            EntityId::name("Mika"),
            EntityId::name("monster_slime_0003"),
            Act::Attack,
        );
        let prose = estimate_tokens(envelope);
        let wire = estimate_tokens(&render_json(&[packet]));
        assert!(
            wire * 3 <= prose,
            "packet {wire} tokens vs envelope {prose} tokens"
        );
    }

    #[test]
    fn a_pipe_in_dialogue_cannot_forge_a_field() {
        let packet = Packet::engage(mika(), saori(), Act::Talk).with_msg("a|b\nc");
        let line = render_compact(&[packet]);
        assert_eq!(line, "C|Mika|Saori|1|a/b c");
        assert_eq!(line.lines().count(), 1);
    }
}

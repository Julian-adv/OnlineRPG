//! Heroic tales (doc/HEROIC_TALES.md): a ledger of true deeds the audit
//! appends to, from which the bard's evening set draws a few to sing.
//! The code picks and rotates; the LLM only embellishes the one line it
//! is handed.

use std::collections::HashMap;
use std::sync::LazyLock;

use rand::seq::SliceRandom;
use rand::Rng;
use serde::Deserialize;
use tracing::warn;

pub const LEDGER_PATH: &str = "data/tales/ledger.txt";

/// Deeds drawn for one evening's performance.
pub const PICKS_PER_NIGHT: usize = 3;

#[derive(Debug, Deserialize)]
struct Named {
    name: String,
}

static MONSTERS: LazyLock<HashMap<String, Named>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../data/monsters.json")).unwrap_or_default()
});
static MAP_LABELS: LazyLock<HashMap<String, Named>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../data/map_labels.json")).unwrap_or_default()
});

fn monster_name(id: &str) -> &str {
    MONSTERS.get(id).map_or(id, |m| m.name.as_str())
}

fn dungeon_name(id: &str) -> &str {
    onlinerpg_shared::dungeon::entrance(id).map_or(id, |d| d.name.as_str())
}

fn place_name(id: &str) -> &str {
    MAP_LABELS.get(id).map_or(id, |l| l.name.as_str())
}

fn item_name(id: &str) -> &str {
    crate::item_defs::get(id).map_or(id, |d| d.name.as_str())
}

/// One ledger line: `DATE KIND NAME [arg | key=value]...`, whitespace
/// separated.
#[derive(Debug, Clone, PartialEq)]
pub struct Deed {
    pub date: String,
    pub kind: String,
    pub name: String,
    args: Vec<String>,
    fields: HashMap<String, String>,
}

impl Deed {
    pub fn parse(line: &str) -> Option<Deed> {
        let mut tokens = line.split_whitespace();
        let date = tokens.next()?.to_string();
        let kind = tokens.next()?.to_string();
        let name = tokens.next()?.to_string();
        let mut args = Vec::new();
        let mut fields = HashMap::new();
        for token in tokens {
            match token.split_once('=') {
                Some((k, v)) => {
                    fields.insert(k.to_string(), v.to_string());
                }
                None => args.push(token.to_string()),
            }
        }
        Some(Deed {
            date,
            kind,
            name,
            args,
            fields,
        })
    }

    fn arg(&self, i: usize) -> Option<&str> {
        self.args.get(i).map(String::as_str)
    }

    fn field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    fn flag(&self, key: &str) -> bool {
        self.field(key) == Some("true")
    }

    pub fn mood(&self) -> &'static str {
        match self.kind.as_str() {
            "enchant_break" | "boss_death" => "tragic",
            _ => "heroic",
        }
    }

    fn in_place(&self) -> String {
        self.arg(1)
            .map_or(String::new(), |d| format!(" in the {}", dungeon_name(d)))
    }

    /// The fact in plain words, for the bard to make a verse of.
    pub fn render(&self) -> String {
        let name = &self.name;
        let mut text = match self.kind.as_str() {
            "boss_kill" => {
                let boss = monster_name(self.arg(0).unwrap_or("a great beast"));
                let alone = if self.flag("solo") { ", alone" } else { "" };
                let mut s = format!("{name} slew the {boss}{alone}{}.", self.in_place());
                if self.flag("first") {
                    s.push_str(" Nobody had ever done it before.");
                }
                s
            }
            "boss_death" => {
                let boss = monster_name(self.arg(0).unwrap_or("a great beast"));
                format!("{name} fell to the {boss}{}.", self.in_place())
            }
            "enchant_up" => {
                let item = item_name(self.arg(0).unwrap_or("a weapon"));
                let plus = self.arg(1).unwrap_or("+?");
                let mut s = format!("{name} enchanted a {item} to {plus}.");
                if self.flag("record") {
                    s.push_str(" No blade in the realm has gone higher.");
                }
                s
            }
            "enchant_break" => {
                let item = item_name(self.arg(0).unwrap_or("a weapon"));
                let plus = self.arg(1).unwrap_or("+?");
                format!("{name}'s {item} shattered on the anvil, reaching past {plus}.")
            }
            "level_record" => {
                let level = self.arg(0).unwrap_or("?");
                let mut s =
                    format!("{name} reached level {level}, higher than anyone in the realm.");
                if let Some((prev, at)) = self.field("prev").and_then(|p| p.split_once(':')) {
                    s.push_str(&format!(" {prev} had held the mark at {at}."));
                }
                s
            }
            "most_levels" => {
                let gained = self.arg(0).unwrap_or("+?").trim_start_matches('+');
                match self.field("reached") {
                    Some(l) => format!(
                        "{name} climbed {gained} levels in a single day, more than anyone, and stands at level {l}."
                    ),
                    None => format!("{name} climbed {gained} levels in a single day, more than anyone."),
                }
            }
            "most_xp" => {
                let mut s = format!("{name} won more experience today than anyone in the realm.");
                if let Some(n) = self.field("streak").and_then(|n| n.parse::<u32>().ok()) {
                    if n > 1 {
                        s.push_str(&format!(" That makes {n} days running."));
                    }
                }
                s
            }
            "farthest" => match self.field("near") {
                Some(place) => format!(
                    "{name} travelled farther from Aldermark than anyone, all the way to {}.",
                    place_name(place)
                ),
                None => format!("{name} travelled farther from Aldermark than anyone."),
            },
            "title" => {
                let title = crate::title_defs::title_name(self.arg(0).unwrap_or("?"));
                format!("{name} earned the title \"{title}\".")
            }
            "rich" => format!("{name}'s coffers now outweigh everyone else's in the realm."),
            other => {
                let rest: Vec<&str> = self
                    .args
                    .iter()
                    .map(String::as_str)
                    .chain(self.fields.values().map(String::as_str))
                    .collect();
                format!("{name}: {other} {}", rest.join(" "))
                    .trim_end()
                    .to_string()
            }
        };
        text.push_str(&format!(" ({})", self.date));
        text
    }
}

/// Read the ledger, oldest first. A missing file is an empty ledger.
pub fn load_ledger(path: &str) -> Vec<Deed> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_ledger(&content)
}

pub fn parse_ledger(content: &str) -> Vec<Deed> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let deed = Deed::parse(l);
            if deed.is_none() {
                warn!("Skipping malformed tale line: {l}");
            }
            deed
        })
        .collect()
}

/// Plain songs between one tale and the next, so late arrivals still hear
/// one and the set is not all stories.
pub const SONGS_BETWEEN_TALES: usize = 2;

/// Tonight's set: a few deeds, one per hero, newer lines favoured, sung in
/// turn and round again, a couple of songs apart.
#[derive(Debug, Default)]
pub struct TonightsTales {
    picks: Vec<Deed>,
    /// Tales told so far tonight: picks the current one and paces the
    /// language alternation.
    sung: usize,
    /// Our song count at which the next tale is due; 0 until the first.
    next_tale_at: usize,
}

impl TonightsTales {
    pub fn draw<R: Rng>(ledger: &[Deed], count: usize, rng: &mut R) -> TonightsTales {
        // Rank weight: the newest line is worth `len` times the oldest.
        let mut pool: Vec<(usize, &Deed)> = ledger.iter().enumerate().collect();
        let mut picks = Vec::new();
        while picks.len() < count && !pool.is_empty() {
            let Ok(&(_, deed)) = pool.choose_weighted(rng, |(i, _)| (*i + 1) as f64) else {
                break;
            };
            let deed = deed.clone();
            pool.retain(|(_, d)| d.name != deed.name);
            picks.push(deed);
        }
        TonightsTales {
            picks,
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.picks.len()
    }

    pub fn current(&self) -> Option<&Deed> {
        self.picks.get(self.sung % self.picks.len().max(1))
    }

    /// The tale to tell now, once our song count has reached its turn.
    pub fn due(&self, songs_started: usize) -> Option<&Deed> {
        if songs_started < self.next_tale_at {
            return None;
        }
        self.current()
    }

    /// Told: the next one waits for this tale's own song plus the gap.
    pub fn advance(&mut self, songs_started: usize) {
        self.sung += 1;
        self.next_tale_at = songs_started + SONGS_BETWEEN_TALES + 1;
    }

    pub fn sung(&self) -> usize {
        self.sung
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Korean,
    English,
}

impl Lang {
    fn name(self) -> &'static str {
        match self {
            Lang::Korean => "Korean",
            Lang::English => "English",
        }
    }
}

fn lang_of_text(text: &str) -> Option<Lang> {
    let hangul = text
        .chars()
        .filter(|c| ('\u{AC00}'..='\u{D7A3}').contains(c))
        .count();
    let latin = text.chars().filter(char::is_ascii_alphabetic).count();
    match hangul.cmp(&latin) {
        std::cmp::Ordering::Greater => Some(Lang::Korean),
        std::cmp::Ordering::Less => Some(Lang::English),
        std::cmp::Ordering::Equal => None,
    }
}

/// What the room speaks, read off the conversation history's `[Chat]`
/// lines (players, not NPCs, not ourselves): `Some` only when every
/// speaker used the same language, `None` for a mixed or silent room.
pub fn audience_lang<'a>(
    history: impl IntoIterator<Item = &'a String>,
    self_name: &str,
) -> Option<Lang> {
    let mut speakers: HashMap<&str, (usize, usize)> = HashMap::new();
    for line in history {
        let Some(rest) = line.split("[Chat] ").nth(1) else {
            continue;
        };
        let Some((name, text)) = rest.split_once(": ") else {
            continue;
        };
        if name == self_name {
            continue;
        }
        let tally = speakers.entry(name).or_default();
        match lang_of_text(text) {
            Some(Lang::Korean) => tally.0 += 1,
            Some(Lang::English) => tally.1 += 1,
            None => {}
        }
    }
    let mut room: Option<Lang> = None;
    for (ko, en) in speakers.values() {
        let lang = match ko.cmp(en) {
            std::cmp::Ordering::Greater => Lang::Korean,
            std::cmp::Ordering::Less => Lang::English,
            std::cmp::Ordering::Equal => continue,
        };
        match room {
            None => room = Some(lang),
            Some(l) if l == lang => {}
            Some(_) => return None,
        }
    }
    room
}

/// The language of the `nth` tale tonight: the room's, when it is of one
/// mind; otherwise Korean and English turn about, Korean first.
pub fn tale_lang(audience: Option<Lang>, nth: usize) -> Lang {
    audience.unwrap_or(if nth.is_multiple_of(2) {
        Lang::Korean
    } else {
        Lang::English
    })
}

/// Prompt section carrying the one deed the bard sings next; the how is
/// in bard.txt's Tales rules.
pub fn prompt_section(deed: &Deed, lang: Lang) -> String {
    format!(
        "\n=== TONIGHT'S TALE (a true deed — sing it as you like) ===\n{}\nHero: {}\nMood: {}\n\
         Language: {} for the opening line and every verse, whatever the listeners spoke.\n\
         Tell it before your next song, as your Tales rules say.\n",
        deed.render(),
        deed.name,
        deed.mood(),
        lang.name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    const LEDGER: &str = "\
# comment
2026-09-01\tboss_kill\tAlder\togre_boss\togre_dungeon\tsolo=false\tfirst=true
2026-09-01  enchant_break  Brann  steel_longsword  +9
2026-09-02  level_record   Cyra   32  prev=Alder:31
2026-09-02  most_levels    Alder  +6  reached=14
2026-09-02  most_xp        Dov    streak=3
2026-09-02  farthest       Eir    near=brovik  dist=6300
2026-09-02  rain_dance     Fenn   wet
garbage
";

    #[test]
    fn tabs_and_spaces_both_parse_and_bad_lines_are_skipped() {
        let deeds = parse_ledger(LEDGER);
        assert_eq!(deeds.len(), 7, "{deeds:?}");
        assert_eq!(deeds[0].name, "Alder");
        assert!(deeds[0].flag("first"));
        assert_eq!(deeds[1].arg(1), Some("+9"));
    }

    #[test]
    fn every_kind_renders_a_sentence_with_the_hero_in_it() {
        for deed in parse_ledger(LEDGER) {
            let text = deed.render();
            assert!(text.contains(&deed.name), "{text}");
            assert!(text.ends_with(&format!("({})", deed.date)), "{text}");
        }
        let deeds = parse_ledger(LEDGER);
        assert!(
            deeds[0].render().contains("Ogre Warlord"),
            "{}",
            deeds[0].render()
        );
        assert!(deeds[0].render().contains("Nobody had ever"));
        assert!(deeds[1].render().contains("shattered"));
        assert_eq!(deeds[1].mood(), "tragic");
        assert_eq!(deeds[0].mood(), "heroic");
        assert!(deeds[4].render().contains("3 days running"));
        assert!(deeds[5].render().contains("Brovik"));
        assert!(
            deeds[6].render().starts_with("Fenn: rain_dance wet"),
            "{}",
            deeds[6].render()
        );
    }

    #[test]
    fn a_draw_takes_one_deed_per_hero_up_to_the_cap() {
        let deeds = parse_ledger(LEDGER);
        let mut rng = StdRng::seed_from_u64(7);
        let night = TonightsTales::draw(&deeds, 3, &mut rng);
        assert_eq!(night.picks.len(), 3);
        let mut names: Vec<&str> = night.picks.iter().map(|d| d.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 3, "{:?}", night.picks);

        let short = TonightsTales::draw(&deeds[..1], 3, &mut rng);
        assert_eq!(short.picks.len(), 1);
        assert!(TonightsTales::draw(&[], 3, &mut rng).current().is_none());
    }

    #[test]
    fn newer_lines_are_favoured() {
        let deeds = parse_ledger(LEDGER);
        let mut rng = StdRng::seed_from_u64(1);
        let mut first_pick_is_old = 0;
        for _ in 0..200 {
            let night = TonightsTales::draw(&deeds, 1, &mut rng);
            if night.picks[0].date == "2026-09-01" {
                first_pick_is_old += 1;
            }
        }
        assert!(
            first_pick_is_old < 60,
            "{first_pick_is_old} of 200 picks were old"
        );
    }

    #[test]
    fn the_room_decides_the_language_and_a_mixed_room_alternates() {
        let ko = |n: &str, t: &str| format!("[19:02] [Chat] {n}: {t}");
        let all_korean = vec![
            ko("민수", "노래 좋네요"),
            ko("jake1", "한 곡 더 부탁해요"),
            "[19:03] [NpcChat] Cocoly: Welcome, traveller!".to_string(),
            ko("Signe", "This one is First Light Waltz"),
        ];
        assert_eq!(audience_lang(&all_korean, "Signe"), Some(Lang::Korean));
        assert_eq!(tale_lang(Some(Lang::Korean), 1), Lang::Korean);

        let all_english = vec![ko("Ann", "play something sad"), ko("Bob", "cheers!")];
        assert_eq!(audience_lang(&all_english, "Signe"), Some(Lang::English));
        assert_eq!(tale_lang(Some(Lang::English), 0), Lang::English);

        let mixed = vec![ko("민수", "노래 좋네요"), ko("Ann", "play something sad")];
        assert_eq!(audience_lang(&mixed, "Signe"), None);
        assert_eq!(tale_lang(None, 0), Lang::Korean);
        assert_eq!(tale_lang(None, 1), Lang::English);
        assert_eq!(tale_lang(None, 2), Lang::Korean);

        let silent: Vec<String> = vec![ko("Signe", "이번에는 First Light Waltz")];
        assert_eq!(
            audience_lang(&silent, "Signe"),
            None,
            "our own lines say nothing"
        );
        assert_eq!(audience_lang(&[ko("Ann", "123 !!")], "Signe"), None);
    }

    #[test]
    fn the_set_rotates_and_wraps() {
        let deeds = parse_ledger(LEDGER);
        let mut rng = StdRng::seed_from_u64(3);
        let mut night = TonightsTales::draw(&deeds, 2, &mut rng);
        let first = night.current().cloned().unwrap();
        assert_eq!(night.sung(), 0);
        night.advance(0);
        assert_eq!(night.sung(), 1);
        assert_ne!(night.current(), Some(&first));
        night.advance(0);
        assert_eq!(night.current(), Some(&first));
        let mut empty = TonightsTales::default();
        empty.advance(0);
        assert!(empty.current().is_none());
    }

    /// A tale, its song, two plain songs, the next tale.
    #[test]
    fn tales_come_two_songs_apart() {
        let deeds = parse_ledger(LEDGER);
        let mut rng = StdRng::seed_from_u64(3);
        let mut night = TonightsTales::draw(&deeds, 2, &mut rng);
        assert!(night.due(5).is_some(), "the first tale waits for nothing");
        night.advance(5);
        assert!(night.due(6).is_none(), "the tale's own song");
        assert!(night.due(7).is_none(), "one plain song");
        assert!(night.due(8).is_some(), "two plain songs");
        assert!(TonightsTales::default().due(9).is_none());
    }
}

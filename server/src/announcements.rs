use axum::{
    body::Bytes,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::OnceCell;
use tracing::{error, warn};

/// Newest N are served; older ones stay on disk but off the login screen.
const MAX_ANNOUNCEMENTS: usize = 50;

/// Cap a single body so one runaway file can't bloat the payload.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Locale for the frontmatter `title` and for body text before any `[xx]`
/// marker. Untagged single-language files stay valid under this default.
const DEFAULT_LOCALE: &str = "ko";

#[derive(Serialize, Clone)]
struct Translation {
    title: String,
    body: String,
}

#[derive(Serialize, Clone)]
pub struct Announcement {
    id: String,
    date: String,
    #[serde(skip)]
    time_secs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    /// locale code -> localized content; always has at least one entry.
    translations: BTreeMap<String, Translation>,
}

pub struct AnnouncementStore {
    dir: PathBuf,
    body: OnceCell<Bytes>,
}

impl AnnouncementStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            body: OnceCell::new(),
        }
    }

    pub async fn warm(&self) {
        let _ = self.body().await;
    }

    /// Announcements are login-screen notices, not game state, so a read
    /// failure deliberately serves an empty list instead of refusing startup.
    /// The failure is not cached: the next request retries the load.
    async fn body(&self) -> Bytes {
        let dir = self.dir.clone();
        self.body
            .get_or_try_init(|| async move {
                let list = tokio::task::spawn_blocking(move || load_announcements(&dir))
                    .await
                    .map_err(|e| {
                        std::io::Error::other(format!("announcement load task panicked: {e}"))
                    })??;
                let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string());
                Ok::<_, std::io::Error>(Bytes::from(json))
            })
            .await
            .cloned()
            .unwrap_or_else(|e| {
                error!("Failed to load announcements, serving none: {e}");
                Bytes::from_static(b"[]")
            })
    }
}

pub fn announcements_router(store: Arc<AnnouncementStore>) -> Router {
    Router::new()
        .route("/api/announcements", get(list_announcements))
        .with_state(store)
}

async fn list_announcements(State(store): State<Arc<AnnouncementStore>>) -> Response {
    let body = store.body().await;
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// Individual unreadable files are skipped so one bad file doesn't hide the
/// rest; an unlistable dir is an error.
fn load_announcements(dir: &Path) -> std::io::Result<Vec<Announcement>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut out = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // `_`-prefixed files (e.g. _README.md) are notes for the operator.
        if stem.starts_with('_') {
            continue;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                warn!("Failed to read announcement {}: {}", path.display(), e);
                continue;
            }
        };
        if let Some(a) = parse_announcement(stem, &raw) {
            out.push(a);
        }
    }

    out.sort_by(newest_first);
    out.truncate(MAX_ANNOUNCEMENTS);
    Ok(out)
}

/// Id tiebreak keeps the order stable across scans.
fn newest_first(a: &Announcement, b: &Announcement) -> std::cmp::Ordering {
    (&b.date, b.time_secs, &b.id).cmp(&(&a.date, a.time_secs, &a.id))
}

/// A file with no resolvable date is skipped (returns None), which is how
/// `_README.md`-style notes stay off the list even if misnamed.
fn parse_announcement(stem: &str, raw: &str) -> Option<Announcement> {
    let (front, body) = split_frontmatter(raw);

    // Frontmatter: shared `date`/`category`, plus `title` (default locale) and
    // `title_<locale>` per-language titles.
    let mut titles: BTreeMap<String, String> = BTreeMap::new();
    let mut date = None;
    let mut time_secs = None;
    let mut category = None;
    for line in front.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let val = val.trim();
        let key = key.trim().to_ascii_lowercase();
        match key.as_str() {
            "date" if is_iso_date(val) => date = Some(val.to_string()),
            "time" => time_secs = parse_time_secs(val).or(time_secs),
            "category" if !val.is_empty() => category = Some(val.to_string()),
            "title" if !val.is_empty() => {
                titles.insert(DEFAULT_LOCALE.to_string(), val.to_string());
            }
            _ => {
                if let Some(loc) = key.strip_prefix("title_").filter(|l| is_locale(l)) {
                    if !val.is_empty() {
                        titles.insert(loc.to_string(), val.to_string());
                    }
                }
            }
        }
    }

    let date = date.or_else(|| date_from_stem(stem))?;

    let bodies = split_body_locales(&body);
    let translations = build_translations(&date, &titles, bodies);
    if translations.is_empty() {
        return None;
    }

    Some(Announcement {
        id: stem.to_string(),
        date,
        time_secs,
        category,
        translations,
    })
}

/// Pairs each locale's body with its title, filling gaps: own title -> first
/// heading of that body -> default-locale title -> the date.
fn build_translations(
    date: &str,
    titles: &BTreeMap<String, String>,
    bodies: BTreeMap<String, String>,
) -> BTreeMap<String, Translation> {
    let mut out = BTreeMap::new();
    for (loc, body) in bodies {
        let title = titles
            .get(&loc)
            .cloned()
            .or_else(|| fallback_title(&body))
            .or_else(|| titles.get(DEFAULT_LOCALE).cloned())
            .unwrap_or_else(|| date.to_string());
        out.insert(loc, Translation { title, body });
    }

    // A title-only announcement (no body sections) still shows in the default
    // locale so short notices don't vanish.
    if out.is_empty() {
        if let Some(title) = titles
            .get(DEFAULT_LOCALE)
            .or_else(|| titles.values().next())
        {
            out.insert(
                DEFAULT_LOCALE.to_string(),
                Translation {
                    title: title.clone(),
                    body: String::new(),
                },
            );
        }
    }
    out
}

/// Splits a body into per-locale sections. Text before the first `[xx]` line
/// belongs to the default locale; each `[xx]` line switches locale. Empty
/// sections are dropped.
fn split_body_locales(body: &str) -> BTreeMap<String, String> {
    let mut sections: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut current = DEFAULT_LOCALE.to_string();
    for line in body.lines() {
        if let Some(loc) = locale_marker(line) {
            current = loc;
            continue;
        }
        sections.entry(current.clone()).or_default().push(line);
    }

    let mut out = BTreeMap::new();
    for (loc, lines) in sections {
        let text = cap_body(lines.join("\n").trim().to_string());
        if !text.is_empty() {
            out.insert(loc, text);
        }
    }
    out
}

/// A line that is exactly `[xx]` (2-3 letters) marks a locale section.
fn locale_marker(line: &str) -> Option<String> {
    let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
    let lower = inner.to_ascii_lowercase();
    is_locale(&lower).then_some(lower)
}

fn is_locale(s: &str) -> bool {
    let len = s.chars().count();
    (2..=3).contains(&len) && s.chars().all(|c| c.is_ascii_alphabetic())
}

fn cap_body(mut body: String) -> String {
    if body.len() > MAX_BODY_BYTES {
        let mut end = MAX_BODY_BYTES;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
    }
    body
}

/// Splits an optional leading `---` frontmatter block from the body. Returns
/// `("", raw)` when there is no well-formed block.
fn split_frontmatter(raw: &str) -> (String, String) {
    let raw = raw.trim_start_matches('\u{feff}');
    let mut lines = raw.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return (String::new(), raw.to_string());
    }

    let mut front = String::new();
    let mut body = Vec::new();
    let mut closed = false;
    for line in lines {
        if !closed && line.trim_end() == "---" {
            closed = true;
            continue;
        }
        if closed {
            body.push(line);
        } else {
            front.push_str(line);
            front.push('\n');
        }
    }

    if closed {
        (front, body.join("\n"))
    } else {
        (String::new(), raw.to_string())
    }
}

fn fallback_title(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('#').trim().to_string())
        .filter(|l| !l.is_empty())
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Two ASCII digits, below `max`.
fn time_part(s: &str, max: u32) -> Option<u32> {
    if s.len() != 2 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok().filter(|v| *v < max)
}

/// `HH:MM` or `HH:MM:SS` as seconds since midnight; a sort key, never served.
fn parse_time_secs(s: &str) -> Option<u32> {
    let mut parts = s.split(':');
    let hour = time_part(parts.next()?, 24)?;
    let minute = time_part(parts.next()?, 60)?;
    let second = parts.next().map_or(Some(0), |p| time_part(p, 60))?;
    parts
        .next()
        .is_none()
        .then_some(hour * 3600 + minute * 60 + second)
}

fn date_from_stem(stem: &str) -> Option<String> {
    let prefix: String = stem.chars().take(10).collect();
    is_iso_date(&prefix).then_some(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr<'a>(a: &'a Announcement, loc: &str) -> &'a Translation {
        a.translations.get(loc).expect("locale present")
    }

    #[test]
    fn parses_single_language() {
        let raw = "---\ntitle: Big Update\ndate: 2026-07-21\ncategory: update\n---\nBody line one.\nLine two.";
        let a = parse_announcement("2026-07-21-x", raw).expect("parsed");
        assert_eq!(a.date, "2026-07-21");
        assert_eq!(a.category.as_deref(), Some("update"));
        assert_eq!(a.translations.len(), 1);
        assert_eq!(tr(&a, "ko").title, "Big Update");
        assert_eq!(tr(&a, "ko").body, "Body line one.\nLine two.");
    }

    #[test]
    fn parses_two_languages() {
        let raw = "---\ntitle: 던전 업데이트\ntitle_en: Dungeon Update\ndate: 2026-07-21\n---\n한국어 본문\n[en]\nEnglish body";
        let a = parse_announcement("2026-07-21-x", raw).expect("parsed");
        assert_eq!(a.translations.len(), 2);
        assert_eq!(tr(&a, "ko").title, "던전 업데이트");
        assert_eq!(tr(&a, "ko").body, "한국어 본문");
        assert_eq!(tr(&a, "en").title, "Dungeon Update");
        assert_eq!(tr(&a, "en").body, "English body");
    }

    #[test]
    fn english_title_falls_back_to_its_own_heading() {
        let raw = "---\ndate: 2026-01-02\n---\n한국어\n[en]\n# English Heading\nbody";
        let a = parse_announcement("note", raw).expect("parsed");
        assert_eq!(tr(&a, "en").title, "English Heading");
    }

    #[test]
    fn date_falls_back_to_filename() {
        let a = parse_announcement("2026-01-02-hello", "no frontmatter here").expect("parsed");
        assert_eq!(a.date, "2026-01-02");
        assert_eq!(tr(&a, "ko").title, "no frontmatter here");
        assert!(a.category.is_none());
    }

    #[test]
    fn undated_file_is_skipped() {
        assert!(parse_announcement("README", "just notes, no date").is_none());
    }

    #[test]
    fn frontmatter_date_overrides_filename() {
        let raw = "---\ndate: 2026-05-05\n---\nbody";
        let a = parse_announcement("2020-01-01-old", raw).expect("parsed");
        assert_eq!(a.date, "2026-05-05");
    }

    #[test]
    fn parses_time_as_seconds() {
        let raw = "---\ndate: 2026-07-22\ntime: 14:19\n---\nbody";
        let a = parse_announcement("x", raw).expect("parsed");
        assert_eq!(a.time_secs, Some(14 * 3600 + 19 * 60));
    }

    #[test]
    fn rejects_invalid_time() {
        for val in ["24:00", "12:60", "14-30", "9:00", "14:30:", "14:30:00:00"] {
            let raw = format!("---\ndate: 2026-07-22\ntime: {val}\n---\nbody");
            let a = parse_announcement("x", &raw).expect("parsed");
            assert!(a.time_secs.is_none(), "{val} should be rejected");
        }
    }

    #[test]
    fn sorts_same_day_by_latest_time() {
        let mut list = [
            parse_announcement("2026-07-22-z", "---\ntime: 09:00\n---\nold").expect("parsed"),
            parse_announcement("2026-07-22-a", "---\ntime: 14:00\n---\nnew").expect("parsed"),
        ];

        list.sort_by(newest_first);

        assert_eq!(list[0].id, "2026-07-22-a");
    }

    #[tokio::test]
    async fn store_loads_announcements_only_once() {
        let dir =
            std::env::temp_dir().join(format!("onlinerpg-announcements-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).expect("created temp directory");
        let path = dir.join("2026-07-22-notice.md");
        std::fs::write(&path, "first body").expect("wrote first announcement");
        let store = AnnouncementStore::new(dir.clone());

        store.warm().await;
        let first = store.body().await;
        std::fs::write(&path, "second body").expect("updated announcement");
        let second = store.body().await;

        std::fs::remove_dir_all(dir).expect("removed temp directory");
        assert_eq!(first, second);
        assert!(String::from_utf8_lossy(&second).contains("first body"));
    }

    #[tokio::test]
    async fn unlistable_dir_serves_empty_without_caching_the_failure() {
        let base = crate::test_util::unique_temp_dir("ann_fail");
        std::fs::create_dir(&base).expect("created temp directory");
        let dir = base.join("announcements");
        std::fs::write(&dir, "not a directory").expect("wrote blocking file");
        let store = AnnouncementStore::new(dir.clone());

        store.warm().await;
        let failed = store.body().await;
        std::fs::remove_file(&dir).expect("removed blocking file");
        std::fs::create_dir(&dir).expect("created announcements dir");
        std::fs::write(dir.join("2026-07-30-notice.md"), "hello").expect("wrote announcement");
        let recovered = store.body().await;

        std::fs::remove_dir_all(base).expect("removed temp directory");
        assert_eq!(&failed[..], b"[]");
        assert!(String::from_utf8_lossy(&recovered).contains("hello"));
    }

    #[test]
    fn missing_dir_loads_empty() {
        let dir = crate::test_util::unique_temp_dir("ann_none");
        assert!(load_announcements(&dir)
            .expect("missing dir is empty")
            .is_empty());
    }

    #[test]
    fn bracketed_body_line_is_not_a_marker() {
        let raw = "---\ndate: 2026-01-02\n---\nsee [link] here";
        let a = parse_announcement("x", raw).expect("parsed");
        assert_eq!(a.translations.len(), 1);
        assert_eq!(tr(&a, "ko").body, "see [link] here");
    }
}

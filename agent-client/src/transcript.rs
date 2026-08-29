//! Full LLM prompt/response transcripts, one append-only file per NPC per
//! UTC day, pruned after `keep_days`. The journal gets one summary line per
//! turn; the full text stays inspectable on disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tracing::{info, warn};

use crate::driver::LlmBackend;

pub struct Transcript {
    dir: PathBuf,
    keep: Duration,
}

impl Transcript {
    /// Empty `dir` leaves transcripts off. Spawns the hourly pruner.
    pub fn start(dir: &str, keep_days: u64) -> Option<Arc<Self>> {
        if dir.is_empty() {
            return None;
        }
        let t = Arc::new(Self {
            dir: PathBuf::from(dir),
            keep: Duration::from_secs(keep_days.max(1) * 86_400),
        });
        let pruner = Arc::clone(&t);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3600));
            loop {
                tick.tick().await;
                let p = Arc::clone(&pruner);
                tokio::task::spawn_blocking(move || p.prune());
            }
        });
        Some(t)
    }

    fn append(&self, label: &str, summary: &str, prompt: &str, body: &str) -> std::io::Result<()> {
        use std::io::Write;
        let dir = self.dir.join(sanitize(label));
        std::fs::create_dir_all(&dir)?;
        let (date, time) = utc_stamp(SystemTime::now());
        let block = format!("==== {date}T{time}Z {summary} ====\n>>>\n{prompt}\n<<<\n{body}\n\n");
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{date}.log")))?
            .write_all(block.as_bytes())
    }

    fn prune(&self) {
        let now = SystemTime::now();
        let Ok(npcs) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for npc in npcs.flatten() {
            let Ok(files) = std::fs::read_dir(npc.path()) else {
                continue;
            };
            for f in files.flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "log") && expired(&p, now, self.keep) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
}

/// Logs one `llm turn` summary line per call and, when transcripts are on,
/// appends the full exchange to the NPC's daily file.
pub struct TranscriptBackend {
    inner: Arc<dyn LlmBackend>,
    transcript: Option<Arc<Transcript>>,
    label: String,
}

impl TranscriptBackend {
    pub fn wrap(
        inner: Arc<dyn LlmBackend>,
        transcript: Option<Arc<Transcript>>,
        label: &str,
    ) -> Arc<dyn LlmBackend> {
        Arc::new(Self {
            inner,
            transcript,
            label: label.to_string(),
        })
    }
}

#[async_trait]
impl LlmBackend for TranscriptBackend {
    async fn send_message(&self, content: &str) -> anyhow::Result<String> {
        let wait = crate::llm_scheduler::queue_wait().unwrap_or_default();
        let started = std::time::Instant::now();
        let result = self.inner.send_message(content).await;
        let (status, body) = match &result {
            Ok(r) => (format!("reply={}B", r.len()), r.clone()),
            Err(e) => ("error".to_string(), format!("{e:#}")),
        };
        let summary = format!(
            "npc={} prompt={}B {status} wait={:.1}s latency={:.2}s",
            self.label,
            content.len(),
            wait.as_secs_f64(),
            started.elapsed().as_secs_f64()
        );
        match &result {
            Ok(_) => info!("llm turn {summary}"),
            Err(_) => info!("llm turn {summary}: {body}"),
        }
        if let Some(t) = &self.transcript {
            let t = Arc::clone(t);
            let label = self.label.clone();
            let prompt = content.to_string();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = t.append(&label, &summary, &prompt, &body) {
                    warn!("transcript write failed for '{label}': {e}");
                }
            });
        }
        result
    }
}

fn expired(p: &Path, now: SystemTime, keep: Duration) -> bool {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| now.duration_since(m).ok())
        .is_some_and(|age| age > keep)
}

fn sanitize(label: &str) -> String {
    let s: String = label
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim_matches('.').to_string();
    if s.is_empty() {
        "_".to_string()
    } else {
        s
    }
}

/// (`YYYY-MM-DD`, `HH:MM:SS`) in UTC, no chrono dependency.
fn utc_stamp(now: SystemTime) -> (String, String) {
    let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let s = secs % 86_400;
    (
        format!("{y:04}-{m:02}-{d:02}"),
        format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60),
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(20_694), (2026, 8, 29));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn stamp_format() {
        let t = UNIX_EPOCH + Duration::from_secs(20_694 * 86_400 + 5 * 3600 + 12 * 60 + 33);
        assert_eq!(utc_stamp(t), ("2026-08-29".into(), "05:12:33".into()));
    }

    #[test]
    fn append_writes_daily_file() {
        let dir = std::env::temp_dir().join(format!("transcript-test-{}", std::process::id()));
        let t = Transcript {
            dir: dir.clone(),
            keep: Duration::from_secs(86_400),
        };
        t.append("Rica", "npc=Rica prompt=5B reply=2B", "hello", "hi")
            .unwrap();
        t.append("Rica", "npc=Rica prompt=5B error", "again", "boom")
            .unwrap();
        let (date, _) = utc_stamp(SystemTime::now());
        let text = std::fs::read_to_string(dir.join("Rica").join(format!("{date}.log"))).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(text.contains("Z npc=Rica prompt=5B reply=2B ====\n>>>\nhello\n<<<\nhi\n"));
        assert!(text.contains("Z npc=Rica prompt=5B error ====\n>>>\nagain\n<<<\nboom\n"));
    }

    #[test]
    fn sanitize_labels() {
        assert_eq!(sanitize("Rica"), "Rica");
        assert_eq!(sanitize("../a/b"), "_a_b");
        assert_eq!(sanitize("선녀"), "선녀");
        assert_eq!(sanitize(""), "_");
    }
}

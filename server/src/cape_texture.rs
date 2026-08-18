//! Player-uploaded cape textures (doc/CAPE_CUSTOMIZATION.md stage 2).
//!
//! Uploads are content-addressed: the server re-decodes and re-encodes every
//! image, so what lands on disk is a PNG the server itself wrote, and the same
//! picture from a hundred players is one file. Serving is public and
//! immutable; blocking is by hash, which is why re-uploading a blocked image
//! cannot smuggle it back in.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::{codecs::png::PngEncoder, ImageEncoder, ImageReader, RgbaImage};
use onlinerpg_terrain::io::atomic_write;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Edge length the client resizes to; anything larger is refused rather than
/// downscaled, so a client bug can't quietly cost everyone bandwidth.
pub const MAX_TEXTURE_SIZE: u32 = 512;
/// Upload body cap. A 512² PNG of a real emblem is far under this.
pub const MAX_UPLOAD_BYTES: usize = 256 * 1024;
/// Uploads one account may make inside `UPLOAD_WINDOW`.
const UPLOAD_LIMIT: usize = 10;
const UPLOAD_WINDOW: Duration = Duration::from_secs(600);
/// Dilation passes that push colour from opaque pixels into transparent ones.
/// Without it the black RGB most editors leave under alpha=0 bleeds into the
/// emblem's edge as a dark halo.
const ALPHA_BLEED_PASSES: u32 = 4;

#[derive(Debug)]
pub enum UploadError {
    NotSignedIn,
    BadHash,
    TooLarge,
    NotAnImage,
    Blocked,
    RateLimited,
    Io(std::io::Error),
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadError::NotSignedIn => write!(f, "sign in first"),
            UploadError::BadHash => write!(f, "that is not a texture hash"),
            UploadError::TooLarge => write!(f, "image too large"),
            UploadError::NotAnImage => write!(f, "not a readable image"),
            UploadError::Blocked => write!(f, "that image is blocked"),
            UploadError::RateLimited => write!(f, "too many uploads; wait a while"),
            UploadError::Io(e) => write!(f, "could not store the image: {e}"),
        }
    }
}

/// Disk layout, the blocklist, live upload sessions and the rate limiter.
/// One instance, shared by the REST routes and the game state.
pub struct CapeTextureStore {
    dir: PathBuf,
    blocked_path: PathBuf,
    reports_path: PathBuf,
    blocked: RwLock<HashSet<String>>,
    /// Upload token → account name. A player's token is issued when they
    /// authenticate and dropped when the connection ends, so the map is
    /// exactly the set of live sessions — no expiry sweep to get wrong.
    sessions: RwLock<HashMap<String, String>>,
    /// Account → recent upload times, trimmed on every check.
    uploads: RwLock<HashMap<String, Vec<Instant>>>,
}

impl CapeTextureStore {
    pub fn new(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let blocked_path = dir.join("blocked");
        let blocked = match std::fs::read_to_string(&blocked_path) {
            Ok(text) => text
                .lines()
                .map(str::trim)
                .filter(|line| is_texture_hash(line))
                .map(str::to_string)
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            reports_path: dir.join("reports.jsonl"),
            dir,
            blocked_path,
            blocked: RwLock::new(blocked),
            sessions: RwLock::new(HashMap::new()),
            uploads: RwLock::new(HashMap::new()),
        })
    }

    /// Hand a freshly authenticated connection its upload credential. The
    /// Google id token the client already holds expires inside an hour while
    /// sessions run all evening, so REST uploads ride this instead.
    pub async fn open_session(&self, account_name: &str) -> String {
        let token = uuid::Uuid::new_v4().simple().to_string();
        self.sessions
            .write()
            .await
            .insert(token.clone(), account_name.to_string());
        token
    }

    pub async fn close_session(&self, token: &str) {
        self.sessions.write().await.remove(token);
    }

    async fn account_for(&self, token: &str) -> Option<String> {
        self.sessions.read().await.get(token).cloned()
    }

    pub async fn is_blocked(&self, hash: &str) -> bool {
        self.blocked.read().await.contains(hash)
    }

    /// Whether a hash names a texture a cape may wear: well-formed, on disk
    /// and not blocked. Everything a client sends is checked through here —
    /// otherwise `cape_texture` is an arbitrary string other clients turn
    /// into a URL.
    pub async fn is_wearable(&self, hash: &str) -> bool {
        self.may_serve(hash).await
            && tokio::fs::try_exists(self.path(hash))
                .await
                .unwrap_or(false)
    }

    /// The half of `is_wearable` that touches no disk: well-formed and not
    /// blocked. The fetch route uses this and lets its own read answer the
    /// "is it there" half, rather than stat-ing the file it is about to open.
    pub async fn may_serve(&self, hash: &str) -> bool {
        is_texture_hash(hash) && !self.is_blocked(hash).await
    }

    pub fn path(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.png"))
    }

    /// Take an uploaded image and return its content hash. The bytes written
    /// are the server's own re-encode of what it decoded, so a payload that
    /// hides something behind a PNG header loses it here.
    pub async fn store(&self, token: &str, bytes: &[u8]) -> Result<String, UploadError> {
        let Some(account) = self.account_for(token).await else {
            return Err(UploadError::NotSignedIn);
        };
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(UploadError::TooLarge);
        }
        if !self.take_upload_slot(&account).await {
            return Err(UploadError::RateLimited);
        }

        // Decoding, bleeding and re-encoding a 512² image is milliseconds of
        // CPU. Rare per player, but this runs on the shared runtime that also
        // carries every other player's ticks, so it goes to a blocking thread.
        let owned = bytes.to_vec();
        let (png, hash) = tokio::task::spawn_blocking(move || {
            let png = reencode(&owned)?;
            let hash = hex_digest(&png);
            Ok::<_, UploadError>((png, hash))
        })
        .await
        .map_err(|e| UploadError::Io(std::io::Error::other(e)))??;

        if self.is_blocked(&hash).await {
            return Err(UploadError::Blocked);
        }
        let path = self.path(&hash);
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            // Atomic, so a torn write never sits under a name that promises
            // its own content.
            atomic_write(&path, &png).await.map_err(UploadError::Io)?;
        }
        Ok(hash)
    }

    async fn take_upload_slot(&self, account: &str) -> bool {
        let now = Instant::now();
        let mut uploads = self.uploads.write().await;
        // Swept whole, not just this account's row: reaching here is itself
        // rate-limited, so the map never outgrows the accounts uploading now.
        uploads.retain(|_, recent| {
            recent.retain(|at| now.duration_since(*at) < UPLOAD_WINDOW);
            !recent.is_empty()
        });
        let recent = uploads.entry(account.to_string()).or_default();
        if recent.len() >= UPLOAD_LIMIT {
            return false;
        }
        recent.push(now);
        true
    }

    /// Record a player's complaint about a texture for an admin to read.
    /// Appended as one JSON line per report; nothing is hidden automatically.
    pub async fn record_report(&self, hash: &str, reporter: &str, target: &str) {
        let report = Report {
            hash,
            reporter,
            target,
            at: crate::auth::unix_now(),
        };
        match serde_json::to_string(&report) {
            Ok(line) => {
                // Off the runtime: a report is one open/append/close, and the
                // thread it would block also carries everyone's ticks.
                let path = self.reports_path.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Err(e) = append_line(&path, &format!("{line}\n")) {
                        warn!("Could not record cape texture report: {e}");
                    }
                })
                .await;
            }
            Err(e) => warn!("Could not encode cape texture report: {e}"),
        }
        info!("Cape texture {hash} reported by {reporter} against {target}");
    }

    /// Stop serving a hash and refuse it on re-upload. Capes still name it,
    /// but the fetch 404s and those capes fall back to their dye.
    pub async fn block(&self, hash: &str) -> Result<(), UploadError> {
        if !is_texture_hash(hash) {
            return Err(UploadError::BadHash);
        }
        if !self.blocked.write().await.insert(hash.to_string()) {
            return Ok(());
        }
        append_line(&self.blocked_path, &format!("{hash}\n")).map_err(UploadError::Io)?;
        match std::fs::remove_file(self.path(hash)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(UploadError::Io(e)),
        }
        info!("Cape texture {hash} blocked");
        Ok(())
    }
}

/// A 64-hex-digit content hash and nothing else — the shape every path,
/// blocklist entry and worn `cape_texture` has to have.
pub fn is_texture_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// One line of `reports.jsonl`.
#[derive(Serialize)]
struct Report<'a> {
    hash: &'a str,
    reporter: &'a str,
    target: &'a str,
    at: i64,
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Decode, bleed the edges and write the PNG back out ourselves.
fn reencode(bytes: &[u8]) -> Result<Vec<u8>, UploadError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| UploadError::NotAnImage)?;
    let decoded = reader.decode().map_err(|_| UploadError::NotAnImage)?;
    if decoded.width() > MAX_TEXTURE_SIZE || decoded.height() > MAX_TEXTURE_SIZE {
        return Err(UploadError::TooLarge);
    }
    let mut image = decoded.to_rgba8();
    bleed_alpha(&mut image);

    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| UploadError::NotAnImage)?;
    Ok(out)
}

/// Push the colour of opaque pixels outwards into transparent ones, leaving
/// alpha alone. The cloth shader mixes by alpha, so a transparent pixel's RGB
/// still shows at the emblem's edge where the filter blends the two.
fn bleed_alpha(image: &mut RgbaImage) {
    let (w, h) = (image.width() as i32, image.height() as i32);
    // One scratch buffer for every pass: the passes read the previous state
    // whole, so they need a copy, but they do not each need a fresh one.
    let mut source = image.clone();
    for _ in 0..ALPHA_BLEED_PASSES {
        source.as_mut().copy_from_slice(image.as_raw());
        let mut changed = false;
        for y in 0..h {
            for x in 0..w {
                if source.get_pixel(x as u32, y as u32).0[3] != 0 {
                    continue;
                }
                let mut sum = [0u32; 3];
                let mut n = 0u32;
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let p = source.get_pixel(nx as u32, ny as u32).0;
                    // Only pixels that already carry colour, so the bleed
                    // spreads outwards one ring per pass instead of averaging
                    // the black it is meant to replace.
                    if p[3] == 0 && p[..3] == [0, 0, 0] {
                        continue;
                    }
                    sum[0] += p[0] as u32;
                    sum[1] += p[1] as u32;
                    sum[2] += p[2] as u32;
                    n += 1;
                }
                if n == 0 {
                    continue;
                }
                let target = image.get_pixel_mut(x as u32, y as u32);
                target.0[0] = (sum[0] / n) as u8;
                target.0[1] = (sum[1] / n) as u8;
                target.0[2] = (sum[2] / n) as u8;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn store(name: &str) -> CapeTextureStore {
        CapeTextureStore::new(crate::test_util::unique_temp_dir(name)).expect("store")
    }

    fn png(size: u32) -> Vec<u8> {
        crate::test_util::test_png(size, [200, 40, 40, 255])
    }

    #[tokio::test]
    async fn an_upload_needs_a_live_session() {
        let store = store("session");
        let bytes = png(8);

        assert!(matches!(
            store.store("nonsense", &bytes).await,
            Err(UploadError::NotSignedIn)
        ));

        let token = store.open_session("account").await;
        assert!(store.store(&token, &bytes).await.is_ok());

        store.close_session(&token).await;
        assert!(matches!(
            store.store(&token, &bytes).await,
            Err(UploadError::NotSignedIn)
        ));
    }

    #[tokio::test]
    async fn a_payload_that_is_not_an_image_is_refused() {
        let store = store("not_an_image");
        let token = store.open_session("account").await;

        assert!(matches!(
            store.store(&token, b"<script>everywhere</script>").await,
            Err(UploadError::NotAnImage)
        ));
    }

    #[tokio::test]
    async fn an_oversized_image_is_refused() {
        let store = store("oversized");
        let token = store.open_session("account").await;
        let bytes = png(MAX_TEXTURE_SIZE + 1);

        assert!(matches!(
            store.store(&token, &bytes).await,
            Err(UploadError::TooLarge)
        ));
    }

    /// Content addressing is the whole storage plan: a picture a hundred
    /// players wear is one file.
    #[tokio::test]
    async fn the_same_picture_stores_once() {
        let store = store("dedup");
        let one = store.open_session("first").await;
        let two = store.open_session("second").await;
        let bytes = png(16);

        let first = store.store(&one, &bytes).await.expect("stored");
        let second = store.store(&two, &bytes).await.expect("stored");

        assert_eq!(first, second);
        assert!(is_texture_hash(&first));
        let files = std::fs::read_dir(&store.dir)
            .expect("dir")
            .filter(|e| {
                e.as_ref()
                    .is_ok_and(|e| e.path().extension().is_some_and(|ext| ext == "png"))
            })
            .count();
        assert_eq!(files, 1);
    }

    #[tokio::test]
    async fn a_blocked_picture_cannot_be_uploaded_back_in() {
        let store = store("reupload");
        let token = store.open_session("account").await;
        let bytes = png(16);
        let hash = store.store(&token, &bytes).await.expect("stored");

        store.block(&hash).await.expect("blocked");

        assert!(matches!(
            store.store(&token, &bytes).await,
            Err(UploadError::Blocked)
        ));
    }

    #[tokio::test]
    async fn the_rate_limit_bites() {
        let store = store("rate_limit");
        let token = store.open_session("account").await;

        for i in 0..UPLOAD_LIMIT {
            let bytes = png(4 + i as u32);
            assert!(store.store(&token, &bytes).await.is_ok());
        }

        let bytes = png(64);
        assert!(matches!(
            store.store(&token, &bytes).await,
            Err(UploadError::RateLimited)
        ));
    }

    /// The cloth shader mixes towards the texture's RGB by its alpha, so the
    /// black most editors leave under alpha=0 would ring the emblem in a dark
    /// halo where the sampler blends the two.
    #[test]
    fn transparent_black_takes_its_neighbour_s_colour() {
        let mut image = RgbaImage::from_pixel(3, 3, Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 1, Rgba([200, 40, 40, 255]));

        bleed_alpha(&mut image);

        let edge = image.get_pixel(0, 1);
        assert_eq!(edge.0[3], 0, "alpha is left alone");
        assert_eq!(edge.0[..3], [200, 40, 40], "the colour bleeds outwards");
    }
}

//! Self-update from GitHub releases: check at startup, download a newer
//! build, swap the binary and shipped data files for the next run.
//! User-edited files (config.toml, user_prompt.txt, memory, cache) are
//! never touched.

use anyhow::Context;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{error, info, warn};

const RELEASES_PAGE: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases");

fn releases_api_url() -> String {
    let repo = env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/");
    format!("https://api.github.com/repos/{repo}/releases?per_page=10")
}
const TAG_PREFIX: &str = "agent-client-v";
/// Written after a successful install. When the latest tag equals this, the
/// update already happened — without it, a release whose Cargo version was
/// not bumped would re-download on every start.
const MARKER_PATH: &str = "data/cache/installed_update_tag";
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Remove the previous binary left behind by a self-update (Windows cannot
/// delete a running exe, so the updater renames it aside instead).
pub fn cleanup_old_binary() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_binary_path(&exe));
    }
}

/// Never fails the program: any error just means running the current build.
pub async fn check() {
    if let Err(e) = try_check().await {
        warn!("Update check skipped: {e:#}");
    }
}

async fn try_check() -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("agent-client/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let releases: Vec<Release> = client
        .get(releases_api_url())
        .timeout(CHECK_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parsing GitHub releases")?;

    let current = parse_version(env!("CARGO_PKG_VERSION")).context("own version unparsable")?;
    let Some(release) = releases.iter().find(|r| {
        r.tag_name
            .strip_prefix(TAG_PREFIX)
            .and_then(parse_version)
            .is_some_and(|v| v > current)
    }) else {
        info!("Up to date (v{})", env!("CARGO_PKG_VERSION"));
        return Ok(());
    };
    let tag = &release.tag_name;

    if std::fs::read_to_string(MARKER_PATH).is_ok_and(|m| m.trim() == tag) {
        warn!("{tag} was already installed; still running v{} — restart, or the release forgot its version bump", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let Some(asset) = release.assets.iter().find(|a| matches_platform(&a.name)) else {
        anyhow::bail!("{tag} has no asset for this platform");
    };

    println!(
        "새 버전 {tag} 이 있습니다 (현재 v{}).",
        env!("CARGO_PKG_VERSION")
    );
    if !confirm("지금 업데이트할까요?") {
        // Headless runs cannot answer; either way the server may refuse an
        // outdated protocol, and the operator updates by hand.
        error!("업데이트를 건너뜁니다. 수동 다운로드: {RELEASES_PAGE}");
        return Ok(());
    }
    info!("Downloading {tag} ({})", asset.name);

    install(&client, asset).await?;
    std::fs::create_dir_all(Path::new(MARKER_PATH).parent().unwrap())?;
    std::fs::write(MARKER_PATH, tag)?;

    println!("업데이트 {tag} 설치 완료 — 다시 실행하세요.");
    std::process::exit(0);
}

fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.split('.').map(|p| p.parse::<u64>().ok());
    match (it.next(), it.next(), it.next(), it.next()) {
        (Some(Some(a)), Some(Some(b)), Some(Some(c)), None) => Some((a, b, c)),
        _ => None,
    }
}

fn matches_platform(name: &str) -> bool {
    let arch = std::env::consts::ARCH;
    if cfg!(windows) {
        name.contains(arch) && name.ends_with("windows-msvc.zip")
    } else {
        name.contains(arch) && name.contains("glibc") && name.ends_with(".tar.gz")
    }
}

fn confirm(question: &str) -> bool {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return false;
    }
    print!("{question} [Y/n] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    !matches!(line.trim(), "n" | "N" | "no")
}

async fn install(client: &reqwest::Client, asset: &Asset) -> anyhow::Result<()> {
    let work_dir = PathBuf::from("data/cache/update");
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir)?;

    let bytes = client
        .get(&asset.browser_download_url)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await
        .context("downloading release asset")?;

    extract(&bytes, &work_dir)?;
    let root = package_root(&work_dir)?;

    replace_binary(&root)?;
    copy_shipped_data(&root)?;

    let _ = std::fs::remove_dir_all(&work_dir);
    Ok(())
}

#[cfg(unix)]
fn extract(bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    tar::Archive::new(flate2::read::GzDecoder::new(bytes)).unpack(dest)?;
    Ok(())
}

#[cfg(windows)]
fn extract(bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    zip::ZipArchive::new(std::io::Cursor::new(bytes))?.extract(dest)?;
    Ok(())
}

/// The archive holds one top-level `agent-client-*` directory.
fn package_root(extracted: &Path) -> anyhow::Result<PathBuf> {
    let mut dirs = std::fs::read_dir(extracted)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir());
    match (dirs.next(), dirs.next()) {
        (Some(only), None) => Ok(only.path()),
        _ => anyhow::bail!("unexpected archive layout"),
    }
}

const BINARY_NAME: &str = if cfg!(windows) {
    concat!(env!("CARGO_PKG_NAME"), ".exe")
} else {
    env!("CARGO_PKG_NAME")
};

fn old_binary_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    exe.with_file_name(name)
}

/// A running binary cannot be overwritten on Windows, but it can be renamed:
/// move ourselves aside, put the new one in our place. The `.old` leftover
/// is deleted on the next start.
fn replace_binary(root: &Path) -> anyhow::Result<()> {
    let new_binary = root.join(BINARY_NAME);
    anyhow::ensure!(new_binary.is_file(), "archive has no {BINARY_NAME}");
    let exe = std::env::current_exe()?;
    let old = old_binary_path(&exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).context("renaming current binary aside")?;
    if let Err(e) = std::fs::copy(&new_binary, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(anyhow::Error::new(e).context("installing new binary"));
    }
    Ok(())
}

/// Everything the archive ships except the binary (installed separately)
/// and user-owned files: config.toml, user_prompt.txt — memory and cache
/// are not in the archive and stay untouched too.
const USER_OWNED: &[&str] = &["data/config.toml", "data/user_prompt.txt"];

fn copy_shipped_data(root: &Path) -> anyhow::Result<()> {
    copy_tree(root, root)
}

fn copy_tree(root: &Path, dir: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        let src = entry.path();
        let rel = src.strip_prefix(root)?.to_path_buf();
        if src.is_dir() {
            std::fs::create_dir_all(&rel)?;
            copy_tree(root, &src)?;
        } else if rel != Path::new(BINARY_NAME) && !USER_OWNED.iter().any(|u| rel == Path::new(u)) {
            std::fs::copy(&src, &rel)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert_eq!(parse_version("0.35.0"), Some((0, 35, 0)));
        assert_eq!(parse_version("v0.35.0"), None);
        assert_eq!(parse_version("0.35"), None);
        assert!(parse_version("0.35.0") > parse_version("0.9.9"));
        assert!(parse_version("1.0.0") > parse_version("0.99.99"));
    }

    #[test]
    #[cfg(all(unix, target_arch = "x86_64"))]
    fn picks_linux_asset() {
        assert!(matches_platform(
            "agent-client-v0.35.0-x86_64-glibc_2.39.tar.gz"
        ));
        assert!(!matches_platform(
            "agent-client-v0.35.0-x86_64-windows-msvc.zip"
        ));
        assert!(!matches_platform(
            "agent-client-v0.35.0-aarch64-glibc_2.39.tar.gz"
        ));
    }
}

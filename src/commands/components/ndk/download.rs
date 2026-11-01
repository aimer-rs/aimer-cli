//! Fetching and unpacking the NDK zip.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use sha1::{Digest, Sha1};

fn client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("aimer/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("creating the HTTP client")
}

/// Download `url` as text.
pub(super) async fn fetch_text(url: &str) -> anyhow::Result<String> {
    client()?
        .get(url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .with_context(|| format!("requesting {url}"))?
        .text()
        .await
        .with_context(|| format!("reading {url}"))
}

/// One line of download progress, e.g. `  42%  330/783 MB`.
pub(super) fn progress_line(done: u64, total: u64) -> String {
    const MB: u64 = 1024 * 1024;
    let percent = if total == 0 {
        0
    } else {
        (done * 100 / total).min(100)
    };
    format!("  {percent:>3}%  {}/{} MB", done / MB, total / MB)
}

/// Download `url` into `dest`, checking it against `expected_sha1`.
pub(super) async fn download(
    url: &str,
    dest: &Path,
    expected_size: u64,
    expected_sha1: &str,
) -> anyhow::Result<()> {
    let mut response = client()?
        .get(url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .with_context(|| format!("requesting {url}"))?;
    let total = response.content_length().unwrap_or(expected_size);
    let mut file = File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut hasher = Sha1::new();
    let (mut done, mut shown) = (0u64, String::new());
    while let Some(chunk) = response.chunk().await.context("downloading the NDK")? {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .with_context(|| format!("writing {}", dest.display()))?;
        done += chunk.len() as u64;
        let line = progress_line(done, total);
        if line != shown {
            print!("\r{line}");
            let _ = std::io::stdout().flush();
            shown = line;
        }
    }
    println!();
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_sha1) {
        bail!("the downloaded NDK is corrupt (SHA-1 {actual}, expected {expected_sha1})");
    }
    Ok(())
}

/// The command that unpacks the zip `archive` into `dest`. `unzip` keeps the
/// symlinks and execute bits the NDK relies on; Windows ships `tar` instead.
pub(super) fn extract_command(archive: &Path, dest: &Path) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("tar");
        cmd.arg("-xf").arg(archive).arg("-C").arg(dest);
        cmd
    } else {
        let mut cmd = Command::new("unzip");
        cmd.arg("-q").arg(archive).arg("-d").arg(dest);
        cmd
    }
}

/// Unpack `archive` into `dest`.
pub(super) fn extract(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let mut cmd = extract_command(archive, dest);
    let program = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .status()
        .with_context(|| format!("running `{program}` (is it installed?)"))?;
    if !status.success() {
        bail!("`{program}` failed to unpack {}", archive.display());
    }
    Ok(())
}

/// The NDK directory inside the freshly unpacked `dir` (`android-ndk-r27c`).
pub(super) fn unpacked_ndk(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("source.properties").is_file())
}

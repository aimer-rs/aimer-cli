//! Talking to GitHub: finding a release and downloading its files.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, bail};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};

use super::release::{REPO, Release, tag_candidates};

fn client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("aimer/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("creating the HTTP client")
}

/// A GET request, authenticated when `GITHUB_TOKEN` or `GH_TOKEN` is set
/// (which lifts GitHub's low anonymous rate limit).
fn get(client: &reqwest::Client, url: &str) -> reqwest::RequestBuilder {
    let request = client.get(url);
    match ["GITHUB_TOKEN", "GH_TOKEN"]
        .into_iter()
        .find_map(|var| std::env::var(var).ok().filter(|token| !token.is_empty()))
    {
        Some(token) => request.bearer_auth(token),
        None => request,
    }
}

/// `Ok(None)` when GitHub answers 404.
async fn fetch_release_at(url: &str) -> anyhow::Result<Option<Release>> {
    let response = get(&client()?, url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;
    match response.status() {
        StatusCode::NOT_FOUND => Ok(None),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
            bail!("GitHub refused the request (rate limit?); set GITHUB_TOKEN and try again")
        }
        status if !status.is_success() => bail!("GitHub answered {status} for {url}"),
        _ => {
            let body = response
                .text()
                .await
                .with_context(|| format!("reading {url}"))?;
            serde_json::from_str(&body)
                .map(Some)
                .with_context(|| format!("reading the release description from {url}"))
        }
    }
}

/// The latest published release (`None`) or the one tagged `requested`.
pub(super) async fn fetch_release(requested: Option<&str>) -> anyhow::Result<Release> {
    let base = format!("https://api.github.com/repos/{REPO}/releases");
    let Some(requested) = requested else {
        return fetch_release_at(&format!("{base}/latest"))
            .await?
            .context("no release has been published yet");
    };
    for tag in tag_candidates(requested) {
        if let Some(release) = fetch_release_at(&format!("{base}/tags/{tag}")).await? {
            return Ok(release);
        }
    }
    bail!("there is no release '{requested}' (see https://github.com/{REPO}/releases)")
}

/// Download `url` as text.
pub(super) async fn fetch_text(url: &str) -> anyhow::Result<String> {
    get(&client()?, url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .with_context(|| format!("requesting {url}"))?
        .text()
        .await
        .with_context(|| format!("reading {url}"))
}

/// One line of download progress, e.g. `  42%  3.1/7.4 MB`.
pub(super) fn progress_line(done: u64, total: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let percent = if total == 0 {
        0
    } else {
        (done * 100 / total).min(100)
    };
    format!(
        "  {percent:>3}%  {:.1}/{:.1} MB",
        done as f64 / MB,
        total as f64 / MB
    )
}

/// Download `url` into `dest`, checking it against `expected_sha256`.
pub(super) async fn download(
    url: &str,
    dest: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    let mut response = get(&client()?, url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .with_context(|| format!("requesting {url}"))?;
    let total = response.content_length().unwrap_or(expected_size);
    let mut file = File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut hasher = Sha256::new();
    let (mut done, mut shown) = (0u64, String::new());
    while let Some(chunk) = response.chunk().await.context("downloading the update")? {
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
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        bail!("the downloaded archive is corrupt (SHA-256 {actual}, expected {expected_sha256})");
    }
    Ok(())
}

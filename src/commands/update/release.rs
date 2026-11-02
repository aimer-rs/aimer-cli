//! What a GitHub release looks like, and which of its files is ours.
//!
//! Releases are published by `.github/workflows/release.yml`: a tag such as
//! `0.10.0` carries `aimer-<tag>-<target>.tar.gz` (`.zip` on Windows) for each
//! supported target, each with a `<archive>.sha256` file beside it.

use anyhow::{Context, bail};
use semver::Version;
use serde::Deserialize;

/// The GitHub repository releases are published to.
pub(super) const REPO: &str = "aimer-rs/aimer-cli";

/// One file attached to a release.
#[derive(Debug, Deserialize)]
pub(super) struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    /// GitHub's own checksum, `sha256:<hex>`.
    #[serde(default)]
    pub digest: Option<String>,
}

/// A published release.
#[derive(Debug, Deserialize)]
pub(super) struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    /// The release's version, read from its tag.
    pub(super) fn version(&self) -> anyhow::Result<Version> {
        parse_version(&self.tag_name)
    }

    pub(super) fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// Parse `0.10.0` or `v0.10.0`.
pub(super) fn parse_version(text: &str) -> anyhow::Result<Version> {
    let text = text.trim();
    Version::parse(text.trim_start_matches('v'))
        .with_context(|| format!("'{text}' is not a version number"))
}

/// The tags a user-supplied version may be published under: releases are
/// tagged without a `v`, but older ones may have it.
pub(super) fn tag_candidates(requested: &str) -> [String; 2] {
    let bare = requested.trim().trim_start_matches('v');
    [bare.to_string(), format!("v{bare}")]
}

/// The Rust target triple a release is built for on this OS and CPU
/// (`std::env::consts` names).
pub(super) fn target_triple(os: &str, arch: &str) -> anyhow::Result<&'static str> {
    Ok(match (os, arch) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        _ => bail!("there is no prebuilt aimer for {os}/{arch}; build it from source instead"),
    })
}

/// The archive's file name for `tag` and `triple`.
pub(super) fn archive_name(tag: &str, triple: &str) -> String {
    let ext = if triple.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("aimer-{tag}-{triple}.{ext}")
}

/// The first hash in a `sha256sum`-style file (`<hex>  <file name>`).
pub(super) fn parse_checksum(text: &str) -> Option<String> {
    let hash = text.split_whitespace().next()?;
    (hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| hash.to_ascii_lowercase())
}

/// GitHub's `sha256:<hex>` digest of `asset`, as bare hex.
pub(super) fn digest_checksum(asset: &Asset) -> Option<String> {
    parse_checksum(asset.digest.as_deref()?.strip_prefix("sha256:")?)
}

/// What to do about the installed version.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Install,
    UpToDate,
    /// The installed build is newer than the latest release.
    Ahead,
}

/// Compare the installed `current` version with the release `target`.
/// `explicit` is a version the user asked for (so a downgrade is allowed);
/// `force` reinstalls even when nothing would change.
pub(super) fn decide(current: &Version, target: &Version, explicit: bool, force: bool) -> Decision {
    if force {
        return Decision::Install;
    }
    match target.cmp(current) {
        std::cmp::Ordering::Greater => Decision::Install,
        std::cmp::Ordering::Equal => Decision::UpToDate,
        std::cmp::Ordering::Less if explicit => Decision::Install,
        std::cmp::Ordering::Less => Decision::Ahead,
    }
}

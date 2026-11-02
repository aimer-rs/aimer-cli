//! `aimer update`: replace this binary with a release from GitHub.
//!
//! The release (the latest one, or the one named with `--version`) is found
//! through the GitHub API, its archive for this OS and CPU is downloaded and
//! checked against the published SHA-256, unpacked, test-run, and only then
//! swapped in for the running binary.

mod github;
mod install;
mod release;
#[cfg(test)]
mod tests;

use anyhow::{Context, bail};

use release::{Decision, REPO, Release};

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `aimer update` was asked to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Only report whether a newer release exists.
    pub check: bool,
    /// Install this release instead of the latest one.
    pub version: Option<String>,
    /// Install even when this version is already installed.
    pub force: bool,
}

pub fn execute(options: &Options) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    runtime.block_on(run(options))
}

async fn run(options: &Options) -> anyhow::Result<()> {
    let current = release::parse_version(CURRENT_VERSION)?;
    println!("Current version: {current}");
    let release = github::fetch_release(options.version.as_deref()).await?;
    let target = release.version()?;
    let explicit = options.version.is_some();

    match release::decide(&current, &target, explicit, options.force) {
        Decision::UpToDate => {
            println!("aimer is up to date ({current}).");
            return Ok(());
        }
        Decision::Ahead => {
            println!("aimer {current} is newer than the latest release ({target}).");
            return Ok(());
        }
        Decision::Install => {}
    }

    if options.check {
        println!("A new version is available: {target}");
        if !release.html_url.is_empty() {
            println!("Release notes: {}", release.html_url);
        }
        println!("Run `aimer update` to install it.");
        return Ok(());
    }

    let exe = install::current_exe()?;
    install::ensure_installed_build(&exe)?;
    update_to(&release, &exe).await?;
    println!("Updated aimer {current} -> {target}.");
    Ok(())
}

/// Download, check and install `release` over the binary at `exe`.
async fn update_to(release: &Release, exe: &std::path::Path) -> anyhow::Result<()> {
    let triple = release::target_triple(std::env::consts::OS, std::env::consts::ARCH)?;
    let archive_name = release::archive_name(&release.tag_name, triple);
    let archive = release.asset(&archive_name).with_context(|| {
        format!(
            "release {} has no {archive_name}; see https://github.com/{REPO}/releases",
            release.tag_name
        )
    })?;
    let checksum = expected_checksum(release, &archive_name).await?;

    let staging = tempfile::tempdir().context("creating a download directory")?;
    let archive_path = staging.path().join(&archive_name);
    println!("Downloading {archive_name}");
    github::download(
        &archive.browser_download_url,
        &archive_path,
        archive.size,
        &checksum,
    )
    .await?;

    let unpacked = staging.path().join("unpacked");
    std::fs::create_dir(&unpacked).context("creating the unpack directory")?;
    install::extract(&archive_path, &unpacked)?;
    let folder = archive_name
        .trim_end_matches(".tar.gz")
        .trim_end_matches(".zip");
    let Some(binary) = install::find_binary(&unpacked, folder) else {
        bail!("{archive_name} does not contain {}", install::binary_name());
    };
    install::verify_runs(&binary)?;
    install::replace(exe, &binary)
}

/// The SHA-256 the release publishes for `archive_name`: its `.sha256` file,
/// else GitHub's digest of the archive.
async fn expected_checksum(release: &Release, archive_name: &str) -> anyhow::Result<String> {
    if let Some(sidecar) = release.asset(&format!("{archive_name}.sha256")) {
        let text = github::fetch_text(&sidecar.browser_download_url).await?;
        if let Some(hash) = release::parse_checksum(&text) {
            return Ok(hash);
        }
    }
    release
        .asset(archive_name)
        .and_then(release::digest_checksum)
        .with_context(|| {
            format!(
                "release {} publishes no checksum for {archive_name}",
                release.tag_name
            )
        })
}

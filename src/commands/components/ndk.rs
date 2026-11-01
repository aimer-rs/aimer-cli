//! The `ndk` component: the Android NDK, downloaded from Google into
//! `~/.aimer/ndk/<version>`.
//!
//! Installing reads Google's SDK repository manifest, asks the user to accept
//! the Android SDK licence (Enter accepts), downloads the zip for this host,
//! checks its SHA-1 and unpacks it. `Ndk::locate` finds the result, so the
//! Android build needs no environment setup afterwards.

mod download;
mod license;
mod manifest;

#[cfg(test)]
mod tests;

use std::io::{self, IsTerminal};
use std::path::Path;

use anyhow::Context;
use colored::Colorize;

use crate::commands::aimer_ndk::{Ndk, NotFound, managed_dir, version_key};
use license::License;
use manifest::Manifest;

/// The versions installed under `root`, oldest first.
pub(super) fn installed(root: &Path) -> Vec<String> {
    let mut versions: Vec<String> = std::fs::read_dir(managed_dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().join("source.properties").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    versions.sort_by_key(|version| version_key(version));
    versions
}

/// Install the NDK matching `wanted` (the newest stable one when `None`).
pub(super) fn install(
    root: &Path,
    wanted: Option<&str>,
    accept_licenses: bool,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;

    println!("Reading Google's Android repository…");
    let xml = runtime.block_on(download::fetch_text(manifest::REPOSITORY_URL))?;
    let manifest = Manifest::parse(&xml, manifest::host_os());
    let release = manifest::pick(&manifest, wanted)?;

    let target = managed_dir(root).join(&release.version);
    if Ndk::at(&target).is_ok() {
        println!("NDK {} is already installed", release.version);
        return Ok(());
    }

    let text = manifest.license_text(&release.license).unwrap_or_default();
    let stdin = io::stdin();
    let interactive = stdin.is_terminal() && io::stdout().is_terminal();
    license::ensure_accepted(
        root,
        &License {
            id: &release.license,
            text,
        },
        accept_licenses,
        interactive,
        &mut stdin.lock(),
        &mut io::stdout(),
    )?;

    let parent = managed_dir(root);
    std::fs::create_dir_all(&parent).with_context(|| format!("creating {}", parent.display()))?;
    // Unpack beside the final location so the last step is a rename, and a
    // failed or interrupted install leaves nothing behind.
    let staging = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(&parent)
        .context("creating a download directory")?;

    let archive = &release.archive;
    println!(
        "{} NDK {} ({} MB)",
        "Downloading".bold(),
        release.version,
        archive.size / (1024 * 1024)
    );
    let zip = staging.path().join("ndk.zip");
    runtime.block_on(download::download(
        &archive.url,
        &zip,
        archive.size,
        &archive.sha1,
    ))?;

    println!("{}", "Unpacking…".bold());
    let unpacked = staging.path().join("unpacked");
    std::fs::create_dir(&unpacked)?;
    download::extract(&zip, &unpacked)?;
    let ndk_dir = download::unpacked_ndk(&unpacked)
        .context("the downloaded archive does not contain an NDK")?;
    let ndk = Ndk::at(&ndk_dir)?;

    let target = parent.join(ndk.version());
    std::fs::rename(&ndk_dir, &target)
        .with_context(|| format!("moving the NDK to {}", target.display()))?;
    println!("Installed NDK {} in {}", ndk.version(), target.display());
    Ok(())
}

/// Remove the installed NDKs matching `wanted` (all of them when `None`),
/// returning the versions that were removed.
pub(super) fn remove(root: &Path, wanted: Option<&str>) -> anyhow::Result<Vec<String>> {
    let mut removed = Vec::new();
    for version in installed(root) {
        if wanted.is_some_and(|wanted| !manifest::version_matches(&version, wanted)) {
            continue;
        }
        let dir = managed_dir(root).join(&version);
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        removed.push(version);
    }
    Ok(removed)
}

/// Find the NDK to build with, offering to install one when there is none.
///
/// The offer is made only in an interactive terminal and only when no NDK can
/// be found at all; a broken `ANDROID_NDK_HOME` is reported as it is.
pub(crate) fn ensure_available() -> anyhow::Result<Ndk> {
    let error = match Ndk::locate() {
        Ok(ndk) => return Ok(ndk),
        Err(error) => error,
    };
    let stdin = io::stdin();
    if !error.is::<NotFound>() || !stdin.is_terminal() || !io::stdout().is_terminal() {
        return Err(error);
    }
    let consent = license::confirm(
        &mut stdin.lock(),
        &mut io::stdout(),
        "Android NDK not found. Press Enter to download and install it into ~/.aimer/ndk, \
         or type 'n' to cancel: ",
    )?;
    if !consent {
        return Err(error);
    }
    install(&super::aimer_root()?, None, false)?;
    Ndk::locate()
}

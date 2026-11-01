//! The licence prompt. Google's NDK terms must be accepted before the download;
//! pressing Enter accepts. An accepted licence is remembered, in the same way
//! `sdkmanager` does, by storing a fingerprint of its text under
//! `~/.aimer/licenses`, so the question is asked again only if the text changes.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use sha1::{Digest, Sha1};

/// A licence as published in Google's manifest.
pub(super) struct License<'a> {
    pub id: &'a str,
    pub text: &'a str,
}

fn fingerprint(text: &str) -> String {
    hex::encode(Sha1::digest(text.as_bytes()))
}

fn record_path(root: &Path, id: &str) -> PathBuf {
    root.join("licenses").join(id)
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether this exact licence text was accepted before.
pub(super) fn is_accepted(root: &Path, license: &License) -> bool {
    std::fs::read_to_string(record_path(root, license.id))
        .is_ok_and(|saved| saved.trim() == fingerprint(license.text))
}

fn record(root: &Path, license: &License) -> anyhow::Result<()> {
    let path = record_path(root, license.id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, fingerprint(license.text))
        .with_context(|| format!("saving the licence record {}", path.display()))
}

/// Ask `question`; Enter, `y` or `yes` is a yes, anything else (or the end of
/// input) is a no.
pub(super) fn confirm(
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    question: &str,
) -> anyhow::Result<bool> {
    write!(out, "{question}")?;
    out.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        writeln!(out)?;
        return Ok(false);
    }
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ))
}

/// Make sure `license` is accepted: already recorded, accepted up front with
/// `assume_yes`, or accepted now by pressing Enter at the prompt.
pub(super) fn ensure_accepted(
    root: &Path,
    license: &License,
    assume_yes: bool,
    interactive: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if !is_safe_id(license.id) {
        bail!("unexpected licence id '{}'", license.id);
    }
    if is_accepted(root, license) {
        return Ok(());
    }
    if assume_yes {
        writeln!(
            out,
            "Accepting the '{}' licence (--accept-licenses).",
            license.id
        )?;
        return record(root, license);
    }
    if !interactive {
        bail!(
            "the Android SDK licence must be accepted before installing the NDK; \
             run this in a terminal, or pass --accept-licenses to accept it"
        );
    }
    writeln!(out, "\n{}\n", license.text)?;
    let accepted = confirm(
        input,
        out,
        "Press Enter to accept the licence above, or type 'n' to decline: ",
    )?;
    if !accepted {
        bail!("the '{}' licence was not accepted", license.id);
    }
    record(root, license)
}

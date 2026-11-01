//! Google's SDK repository manifest, reduced to the NDK releases Aimer can
//! install: stable releases of r23 or newer that have a download for this host.

use std::collections::HashMap;

use anyhow::{Context, bail};

/// The manifest `sdkmanager` reads; it lists every NDK with its checksum.
pub(super) const REPOSITORY_URL: &str =
    "https://dl.google.com/android/repository/repository2-3.xml";

const DOWNLOAD_BASE: &str = "https://dl.google.com/android/repository/";

/// Oldest NDK major version `aimer_ndk` can build with.
const MIN_MAJOR: u32 = 23;

/// The `host-os` value Google uses for the machine Aimer is running on.
pub(super) fn host_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macosx"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// The zip to download for one release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Archive {
    pub url: String,
    pub size: u64,
    pub sha1: String,
}

/// One installable NDK release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Release {
    /// The full version, e.g. `27.3.13750724`; also the install directory name.
    pub version: String,
    revision: (u32, u32, u32),
    /// Id of the licence the user must accept before downloading.
    pub license: String,
    pub archive: Archive,
}

#[derive(Debug)]
pub(super) struct Manifest {
    licenses: HashMap<String, String>,
    releases: Vec<Release>,
}

impl Manifest {
    /// Read the NDK releases that have a download for `host_os`.
    pub(super) fn parse(xml: &str, host_os: &str) -> Self {
        Self {
            licenses: parse_licenses(xml),
            releases: parse_releases(xml, host_os),
        }
    }

    pub(super) fn license_text(&self, id: &str) -> Option<&str> {
        self.licenses.get(id).map(String::as_str)
    }

    /// The newest release matching `wanted` (any release when `None`).
    pub(super) fn select(&self, wanted: Option<&str>) -> anyhow::Result<&Release> {
        self.releases
            .iter()
            .filter(|release| wanted.is_none_or(|wanted| version_matches(&release.version, wanted)))
            .max_by_key(|release| release.revision)
            .with_context(|| match wanted {
                Some(wanted) => format!(
                    "no NDK matches '{wanted}' (available: {})",
                    self.available()
                ),
                None => "Google's repository lists no installable NDK for this platform".into(),
            })
    }

    fn available(&self) -> String {
        let mut releases: Vec<&Release> = self.releases.iter().collect();
        releases.sort_by_key(|release| std::cmp::Reverse(release.revision));
        releases
            .iter()
            .map(|release| release.version.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Whether `version` (`27.3.13750724`) is what the user asked for with
/// `wanted`: the whole version or a leading part of it (`27`, `r27`, `27.3`).
pub(crate) fn version_matches(version: &str, wanted: &str) -> bool {
    let wanted = wanted.trim_start_matches(['r', 'R']);
    !wanted.is_empty() && (version == wanted || version.starts_with(&format!("{wanted}.")))
}

/// The text between the first `open` and the `close` after it.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)?;
    Some(&text[start..start + end])
}

fn number(text: &str, tag: &str) -> Option<u32> {
    between(text, &format!("<{tag}>"), &format!("</{tag}>"))?
        .trim()
        .parse()
        .ok()
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_licenses(xml: &str) -> HashMap<String, String> {
    xml.split("<license id=\"")
        .skip(1)
        .filter_map(|chunk| {
            let (id, rest) = chunk.split_once('"')?;
            let body = between(rest, ">", "</license>")?;
            Some((id.to_string(), unescape(body.trim())))
        })
        .collect()
}

fn parse_releases(xml: &str, host_os: &str) -> Vec<Release> {
    xml.split("<remotePackage path=\"")
        .skip(1)
        .filter_map(|chunk| {
            let (path, rest) = chunk.split_once('"')?;
            let version = path.strip_prefix("ndk;")?;
            let body = rest.split("</remotePackage>").next()?;
            if body.contains("<preview>") {
                return None;
            }
            let revision = (
                number(body, "major")?,
                number(body, "minor")?,
                number(body, "micro")?,
            );
            if revision.0 < MIN_MAJOR {
                return None;
            }
            Some(Release {
                version: version.to_string(),
                revision,
                license: between(body, "<uses-license ref=\"", "\"")?.to_string(),
                archive: body
                    .split("<archive>")
                    .skip(1)
                    .find_map(|archive| parse_archive(archive, host_os))?,
            })
        })
        .collect()
}

fn parse_archive(text: &str, host_os: &str) -> Option<Archive> {
    if between(text, "<host-os>", "</host-os>")?.trim() != host_os {
        return None;
    }
    let url = between(text, "<url>", "</url>")?.trim();
    Some(Archive {
        url: format!("{DOWNLOAD_BASE}{url}"),
        size: between(text, "<size>", "</size>")?.trim().parse().ok()?,
        sha1: between(text, "<checksum type=\"sha1\">", "</checksum>")?
            .trim()
            .to_ascii_lowercase(),
    })
}

/// The release to install, or an error saying why there is none.
pub(super) fn pick<'a>(
    manifest: &'a Manifest,
    wanted: Option<&str>,
) -> anyhow::Result<&'a Release> {
    let release = manifest.select(wanted)?;
    if manifest.license_text(&release.license).is_none() {
        bail!(
            "Google's repository does not include the '{}' licence the NDK requires",
            release.license
        );
    }
    Ok(release)
}

//! Packaging an iOS app into an `.ipa`, and optionally uploading it to App
//! Store Connect.
//!
//! This is Apple's own route, the one Xcode's Organizer follows: the project is
//! *archived* (built and signed for development), then the archive is
//! *exported* for a distribution method, which re-signs it and writes the
//! `.ipa`. Uploading is a second export whose destination is Apple.
//!
//! Settings come from `[ios.signing]` in `Aimer.toml`; the environment
//! overrides them:
//!
//! | Variable                                   | Overrides                    |
//! |--------------------------------------------|------------------------------|
//! | `AIMER_IOS_TEAM_ID`                        | `team_id`                    |
//! | `AIMER_IOS_EXPORT_METHOD`                  | `method`                     |
//! | `AIMER_IOS_ALLOW_PROVISIONING_UPDATES`     | `allow_provisioning_updates` |
//! | `AIMER_ASC_KEY`, `_KEY_ID`, `_ISSUER`      | an App Store Connect API key |
//!
//! The API key is optional when Xcode is signed in to the developer account;
//! CI needs it. All external tools are built as plain [`Command`]s and handed
//! to the [`Reporter`], so the argument lists can be asserted without touching
//! the machine.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use super::sign::non_empty;
use super::{Request, dist_dir, package_stem, reset_dir};
use crate::commands::assemble::{
    self, IOS_PROJECT_DIR, IosPlan, Reporter, StdioReporter, Step, StepKind, profile_name,
    xcode_configuration,
};
use crate::config::{AimerManifest, IosSigningMeta, resolve_cargo_package_name};

const ENV_TEAM_ID: &str = "AIMER_IOS_TEAM_ID";
const ENV_METHOD: &str = "AIMER_IOS_EXPORT_METHOD";
const ENV_PROVISIONING_UPDATES: &str = "AIMER_IOS_ALLOW_PROVISIONING_UPDATES";
const ENV_ASC_KEY: &str = "AIMER_ASC_KEY";
const ENV_ASC_KEY_ID: &str = "AIMER_ASC_KEY_ID";
const ENV_ASC_ISSUER: &str = "AIMER_ASC_ISSUER";

/// The export method that sends builds to App Store Connect.
const APP_STORE_METHOD: &str = "app-store-connect";
/// What `xcodebuild -exportArchive` accepts for an iOS archive, current names
/// only (`app-store`, `ad-hoc` and `development` are deprecated spellings).
const EXPORT_METHODS: [&str; 4] = [
    APP_STORE_METHOD,
    "release-testing",
    "enterprise",
    "debugging",
];

/// Scratch space of one packaging run: the archive and the exports.
const WORK_DIR_NAME: &str = ".ios-work";

/// An App Store Connect API key, the credential `xcodebuild` uses on CI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AscKey {
    pub path: String,
    pub key_id: String,
    pub issuer: String,
}

/// Everything needed to sign and export one iOS build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IosSigningPlan {
    pub team_id: String,
    /// One of [`EXPORT_METHODS`].
    pub method: String,
    /// Whether `xcodebuild` may create and update App IDs, certificates and
    /// profiles in the developer account.
    pub allow_provisioning_updates: bool,
    pub key: Option<AscKey>,
}

impl IosSigningPlan {
    /// Combine the manifest settings with the environment.
    ///
    /// `env` is injected so the resolution can be tested without touching the
    /// process environment.
    ///
    /// # Errors
    ///
    /// Fails, naming the setting and the variable that would provide it, when
    /// the team is missing, the method is unknown, an API key is only partly
    /// given, or an upload is asked for something App Store Connect would not
    /// take (another method, or a debug build).
    pub(super) fn resolve(
        meta: Option<&IosSigningMeta>,
        env: &dyn Fn(&str) -> Option<String>,
        upload: bool,
        release: bool,
    ) -> anyhow::Result<Self> {
        let team_id =
            non_empty(env(ENV_TEAM_ID)).or_else(|| non_empty(meta.and_then(|m| m.team_id.clone())));
        let Some(team_id) = team_id else {
            bail!(
                "no Apple Developer team: set `team_id` under [ios.signing] in Aimer.toml \
                 or the {ENV_TEAM_ID} environment variable"
            );
        };

        let method = non_empty(env(ENV_METHOD))
            .or_else(|| non_empty(meta.and_then(|m| m.method.clone())))
            .unwrap_or_else(|| APP_STORE_METHOD.to_string());
        if !EXPORT_METHODS.contains(&method.as_str()) {
            bail!(
                "unknown export method '{method}'; use one of: {}",
                EXPORT_METHODS.join(", ")
            );
        }
        if upload && method != APP_STORE_METHOD {
            bail!("--upload needs the '{APP_STORE_METHOD}' export method, not '{method}'");
        }
        if upload && !release {
            bail!("App Store Connect takes release builds only; drop --debug to upload");
        }

        let allow_provisioning_updates = match non_empty(env(ENV_PROVISIONING_UPDATES)) {
            Some(value) => matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
            None => meta
                .and_then(|m| m.allow_provisioning_updates)
                .unwrap_or(false),
        };

        Ok(Self {
            team_id,
            method,
            allow_provisioning_updates,
            key: resolve_key(env)?,
        })
    }

    /// Check what can be checked before the (slow) build.
    pub(super) fn preflight(&self) -> anyhow::Result<()> {
        if let Some(key) = &self.key
            && !Path::new(&key.path).is_file()
        {
            bail!("the App Store Connect key '{}' does not exist", key.path);
        }
        Ok(())
    }

    /// The `xcodebuild` options that let it sign: provisioning updates when
    /// allowed, and the API key when there is one.
    fn auth_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if self.allow_provisioning_updates {
            args.push("-allowProvisioningUpdates".to_string());
        }
        if let Some(key) = &self.key {
            args.extend([
                "-authenticationKeyPath".to_string(),
                key.path.clone(),
                "-authenticationKeyID".to_string(),
                key.key_id.clone(),
                "-authenticationKeyIssuerID".to_string(),
                key.issuer.clone(),
            ]);
        }
        args
    }
}

/// The API key from the environment: all three variables or none.
fn resolve_key(env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Option<AscKey>> {
    let path = non_empty(env(ENV_ASC_KEY));
    let key_id = non_empty(env(ENV_ASC_KEY_ID));
    let issuer = non_empty(env(ENV_ASC_ISSUER));
    match (path, key_id, issuer) {
        (None, None, None) => Ok(None),
        (Some(path), Some(key_id), Some(issuer)) => Ok(Some(AscKey {
            path,
            key_id,
            issuer,
        })),
        (path, key_id, issuer) => {
            let missing: Vec<&str> = [
                (ENV_ASC_KEY, path.is_none()),
                (ENV_ASC_KEY_ID, key_id.is_none()),
                (ENV_ASC_ISSUER, issuer.is_none()),
            ]
            .into_iter()
            .filter_map(|(name, absent)| absent.then_some(name))
            .collect();
            bail!(
                "an App Store Connect API key needs all three variables; missing: {}",
                missing.join(", ")
            )
        }
    }
}

/// What to archive and where the results go.
pub(super) struct ArchiveJob<'a> {
    /// The Cargo package, which names the Xcode project and scheme.
    pub pkg_name: &'a str,
    /// The application name, which names the bundle and the package.
    pub app_name: &'a str,
    pub version: &'a str,
    pub release: bool,
    /// Directory the `.ipa` is written to.
    pub dist: &'a str,
    /// Upload to App Store Connect after exporting.
    pub upload: bool,
}

/// Build the iOS app and package it as the request says; returns the `.ipa`.
pub(super) fn run(request: &Request, manifest: Option<&AimerManifest>) -> anyhow::Result<String> {
    let release = request.release;
    let meta = manifest
        .and_then(|m| m.ios.as_ref())
        .and_then(|m| m.signing.as_ref());
    // Checked before the (slow) build, so a missing team or a mistyped key
    // path fails in seconds.
    let plan = IosSigningPlan::resolve(
        meta,
        &|key| std::env::var(key).ok(),
        request.upload,
        release,
    )?;
    plan.preflight()?;

    println!(
        "Packaging 'ios' app into an .ipa ({}) in {} mode{}...",
        plan.method,
        profile_name(release),
        if request.upload {
            ", then uploading it"
        } else {
            ""
        }
    );

    let reporter = StdioReporter;
    let pkg_name = resolve_cargo_package_name(Path::new("."));
    let ios_plan = IosPlan::resolve(false);
    assemble::build_ios(&ios_plan, release, &reporter)?;
    assemble::stage_ios_inputs(&pkg_name, &ios_plan, release, &reporter)?;

    let version = manifest
        .map(|manifest| manifest.package.version.clone())
        .unwrap_or_default();
    let app_name = assemble::app_name();
    let dist = dist_dir(request.target, release);
    let job = ArchiveJob {
        pkg_name: &pkg_name,
        app_name: &app_name,
        version: &version,
        release,
        dist: &dist,
        upload: request.upload,
    };
    export_ipa(&job, &plan, &reporter)
}

/// Archive, export and (optionally) upload; returns the path of the `.ipa`.
pub(super) fn export_ipa(
    job: &ArchiveJob,
    plan: &IosSigningPlan,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    std::fs::create_dir_all(job.dist).with_context(|| format!("creating {}", job.dist))?;
    let work = std::path::absolute(format!("{}/{WORK_DIR_NAME}", job.dist))
        .context("resolving the scratch directory")?;
    reset_dir(&work.to_string_lossy())?;

    let result = archive_export_upload(job, plan, &work, reporter);
    // The archive holds a full copy of the app and its symbols; never leave it
    // behind, whether or not the export worked.
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn archive_export_upload(
    job: &ArchiveJob,
    plan: &IosSigningPlan,
    work: &Path,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    let archive = work.join(format!("{}.xcarchive", job.app_name));
    reporter.note(format!("Archiving {}...", job.app_name));
    reporter.run(
        archive_command(job, plan, &archive),
        Step::new(StepKind::Xcode, "xcodebuild archive for iOS"),
    )?;

    // The archive is signed for development at this point and the export signs
    // it again, so the assets and the version can still be put into the app.
    let app = archive
        .join("Products/Applications")
        .join(format!("{}.app", job.app_name));
    reporter.stage_assets(&app.to_string_lossy())?;
    let version = job.version.trim();
    if !version.is_empty() {
        reporter.run(
            plist_set_command(
                &app.join("Info.plist"),
                "CFBundleShortVersionString",
                version,
            ),
            Step::new(StepKind::Other, "PlistBuddy"),
        )?;
    }

    let export_dir = work.join("export");
    reporter.note(format!("Exporting for {}...", plan.method));
    export(&export_dir, &archive, plan, false, work, reporter)?;
    let ipa = find_ipa(&export_dir)?;

    let output = format!(
        "{}/{}.ipa",
        job.dist,
        package_stem(job.app_name, job.version)
    );
    match std::fs::remove_file(&output) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing stale package {output}")),
    }
    std::fs::copy(&ipa, &output).with_context(|| format!("copying {}", ipa.display()))?;

    if job.upload {
        reporter.note("Uploading to App Store Connect...".to_string());
        export(&work.join("upload"), &archive, plan, true, work, reporter)?;
    }
    Ok(output)
}

/// One `xcodebuild -exportArchive`, locally or straight to App Store Connect.
fn export(
    export_dir: &Path,
    archive: &Path,
    plan: &IosSigningPlan,
    upload: bool,
    work: &Path,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    let plist = work.join(if upload {
        "ExportOptionsUpload.plist"
    } else {
        "ExportOptions.plist"
    });
    std::fs::write(&plist, export_options_plist(plan, upload))
        .with_context(|| format!("writing {}", plist.display()))?;
    let action = if upload {
        "xcodebuild export (upload)"
    } else {
        "xcodebuild export"
    };
    reporter.run(
        export_command(archive, export_dir, &plist, plan),
        Step::new(StepKind::Xcode, action),
    )
}

/// The `.ipa` an export wrote; Xcode names it after the scheme.
fn find_ipa(export_dir: &Path) -> anyhow::Result<PathBuf> {
    let entries = std::fs::read_dir(export_dir)
        .with_context(|| format!("reading the export in {}", export_dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "ipa") {
            return Ok(path);
        }
    }
    bail!("the export in {} contains no .ipa", export_dir.display())
}

/// `xcodebuild archive` of the iOS project for any device.
pub(super) fn archive_command(job: &ArchiveJob, plan: &IosSigningPlan, archive: &Path) -> Command {
    let mut cmd = Command::new("xcodebuild");
    cmd.arg("archive")
        .arg("-project")
        .arg(format!("{}.xcodeproj", job.pkg_name))
        .arg("-scheme")
        .arg(job.pkg_name)
        .arg("-configuration")
        .arg(xcode_configuration(job.release))
        .args(["-destination", "generic/platform=iOS"])
        .arg("-archivePath")
        .arg(archive)
        // The scaffold names the product after its target; the override makes
        // the bundle carry the application name, as in `aimer assemble`.
        .arg(format!("PRODUCT_NAME={}", job.app_name))
        .arg(format!("DEVELOPMENT_TEAM={}", plan.team_id))
        .arg("CODE_SIGN_STYLE=Automatic")
        .args(plan.auth_args())
        .current_dir(IOS_PROJECT_DIR);
    cmd
}

/// `xcodebuild -exportArchive`, which re-signs the app for the export method.
pub(super) fn export_command(
    archive: &Path,
    export_dir: &Path,
    plist: &Path,
    plan: &IosSigningPlan,
) -> Command {
    let mut cmd = Command::new("xcodebuild");
    cmd.arg("-exportArchive")
        .arg("-archivePath")
        .arg(archive)
        .arg("-exportPath")
        .arg(export_dir)
        .arg("-exportOptionsPlist")
        .arg(plist)
        .args(plan.auth_args());
    cmd
}

/// The `ExportOptions.plist` of an export; `upload` sends the build to Apple
/// instead of writing it to disk.
pub(super) fn export_options_plist(plan: &IosSigningPlan, upload: bool) -> String {
    let text = |key: &str, value: &str| {
        format!(
            "\t<key>{key}</key>\n\t<string>{}</string>\n",
            xml_escape(value)
        )
    };
    let mut body = String::new();
    body += &text("method", &plan.method);
    body += &text("teamID", &plan.team_id);
    body += &text("signingStyle", "automatic");
    body += &text("destination", if upload { "upload" } else { "export" });
    if plan.method == APP_STORE_METHOD {
        // Symbols make crash reports from the App Store readable.
        body += "\t<key>uploadSymbols</key>\n\t<true/>\n";
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n"
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Set a string key of an `Info.plist`.
pub(super) fn plist_set_command(plist: &Path, key: &str, value: &str) -> Command {
    let mut cmd = Command::new("/usr/libexec/PlistBuddy");
    cmd.arg("-c").arg(format!("Set :{key} {value}")).arg(plist);
    cmd
}

#[cfg(test)]
mod tests;

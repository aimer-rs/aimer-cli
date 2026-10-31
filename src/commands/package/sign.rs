//! Code signing and notarization of the macOS package.
//!
//! Settings come from `[macos.signing]` in `Aimer.toml`, and any of the
//! `AIMER_*` environment variables below override them. Certificates and
//! notary credentials themselves are never read by Aimer: they stay in the
//! keychain (or, for an App Store Connect key, in a file the variables point
//! to).
//!
//! | Variable                          | Overrides                 |
//! |-----------------------------------|---------------------------|
//! | `AIMER_MACOS_SIGN_IDENTITY`       | `identity`                |
//! | `AIMER_MACOS_INSTALLER_IDENTITY`  | `installer_identity`      |
//! | `AIMER_MACOS_ENTITLEMENTS`        | `entitlements`            |
//! | `AIMER_NOTARY_PROFILE`            | `notary_profile`          |
//! | `AIMER_NOTARY_KEY`, `_KEY_ID`, `_ISSUER` | an API key, instead of a profile |

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

use super::PackageFormat;
use crate::config::MacosSigningMeta;

const ENV_IDENTITY: &str = "AIMER_MACOS_SIGN_IDENTITY";
const ENV_INSTALLER_IDENTITY: &str = "AIMER_MACOS_INSTALLER_IDENTITY";
const ENV_ENTITLEMENTS: &str = "AIMER_MACOS_ENTITLEMENTS";
const ENV_NOTARY_PROFILE: &str = "AIMER_NOTARY_PROFILE";
const ENV_NOTARY_KEY: &str = "AIMER_NOTARY_KEY";
const ENV_NOTARY_KEY_ID: &str = "AIMER_NOTARY_KEY_ID";
const ENV_NOTARY_ISSUER: &str = "AIMER_NOTARY_ISSUER";

/// How `notarytool` authenticates with Apple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum NotaryAuth {
    /// A profile saved with `xcrun notarytool store-credentials`.
    KeychainProfile(String),
    /// An App Store Connect API key, the usual choice on CI.
    ApiKey {
        path: String,
        key_id: String,
        issuer: String,
    },
}

/// Everything needed to sign (and optionally notarize) one package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SigningPlan {
    /// "Developer ID Application" identity for the app and the `.dmg`.
    pub identity: String,
    /// "Developer ID Installer" identity; present for a `.pkg`.
    pub installer_identity: Option<String>,
    /// Entitlements plist for the app, relative to the project root.
    pub entitlements: Option<String>,
    /// Notary credentials; present when the package is to be notarized.
    pub notary: Option<NotaryAuth>,
}

impl SigningPlan {
    /// Combine the manifest settings with the environment.
    ///
    /// `env` is injected so the resolution can be tested without touching the
    /// process environment.
    ///
    /// # Errors
    ///
    /// Fails, naming the setting and the variable that would provide it, when
    /// the identity a package needs or the notary credentials are missing.
    pub(super) fn resolve(
        meta: Option<&MacosSigningMeta>,
        env: &dyn Fn(&str) -> Option<String>,
        format: PackageFormat,
        notarize: bool,
    ) -> anyhow::Result<Self> {
        let setting = |var: &str, manifest: Option<&String>| {
            non_empty(env(var)).or_else(|| non_empty(manifest.cloned()))
        };

        let identity = setting(ENV_IDENTITY, meta.and_then(|m| m.identity.as_ref()));
        let Some(identity) = identity else {
            bail!(
                "no signing identity: set `identity` under [macos.signing] in Aimer.toml \
                 or the {ENV_IDENTITY} environment variable"
            );
        };

        let installer_identity = setting(
            ENV_INSTALLER_IDENTITY,
            meta.and_then(|m| m.installer_identity.as_ref()),
        );
        if format == PackageFormat::Pkg && installer_identity.is_none() {
            bail!(
                "a .pkg needs a \"Developer ID Installer\" identity: set `installer_identity` \
                 under [macos.signing] in Aimer.toml or the {ENV_INSTALLER_IDENTITY} \
                 environment variable"
            );
        }

        let notary = if notarize {
            Some(resolve_notary(meta, env)?)
        } else {
            None
        };

        Ok(Self {
            identity,
            installer_identity: installer_identity.filter(|_| format == PackageFormat::Pkg),
            entitlements: setting(ENV_ENTITLEMENTS, meta.and_then(|m| m.entitlements.as_ref())),
            notary,
        })
    }
}

fn resolve_notary(
    meta: Option<&MacosSigningMeta>,
    env: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<NotaryAuth> {
    let key = non_empty(env(ENV_NOTARY_KEY));
    let key_id = non_empty(env(ENV_NOTARY_KEY_ID));
    let issuer = non_empty(env(ENV_NOTARY_ISSUER));

    match (key, key_id, issuer) {
        (Some(path), Some(key_id), Some(issuer)) => Ok(NotaryAuth::ApiKey {
            path,
            key_id,
            issuer,
        }),
        (None, None, None) => {
            let profile = non_empty(env(ENV_NOTARY_PROFILE))
                .or_else(|| non_empty(meta.and_then(|m| m.notary_profile.clone())));
            match profile {
                Some(profile) => Ok(NotaryAuth::KeychainProfile(profile)),
                None => bail!(
                    "no notary credentials: set `notary_profile` under [macos.signing] in \
                     Aimer.toml (see `xcrun notarytool store-credentials`), the \
                     {ENV_NOTARY_PROFILE} variable, or {ENV_NOTARY_KEY}, {ENV_NOTARY_KEY_ID} and \
                     {ENV_NOTARY_ISSUER} for an App Store Connect API key"
                ),
            }
        }
        (key, key_id, issuer) => {
            let missing: Vec<&str> = [
                (ENV_NOTARY_KEY, key.is_none()),
                (ENV_NOTARY_KEY_ID, key_id.is_none()),
                (ENV_NOTARY_ISSUER, issuer.is_none()),
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

/// `Some` only for a value with visible characters, so an exported but empty
/// variable counts as unset.
pub(super) fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Whether `identity` (a name, a part of one, or a SHA-1) appears in the output
/// of `security find-identity -v`.
pub(super) fn identity_listed(listing: &str, identity: &str) -> bool {
    listing.contains(identity)
}

/// Check, before anything is built, that the signing inputs exist.
///
/// A missing certificate or a wrong notary profile otherwise only surfaces
/// after the build, or after minutes of waiting for Apple.
pub(super) fn preflight(plan: &SigningPlan) -> anyhow::Result<()> {
    let listing = Command::new("security")
        .args(["find-identity", "-v"])
        .output()
        .context("running `security find-identity`")?;
    let listing = String::from_utf8_lossy(&listing.stdout);

    require_identity(&listing, &plan.identity)?;
    if let Some(installer) = &plan.installer_identity {
        require_identity(&listing, installer)?;
    }
    if let Some(path) = &plan.entitlements
        && !Path::new(path).is_file()
    {
        bail!("entitlements file '{path}' does not exist");
    }
    if let Some(auth) = &plan.notary {
        let status = Command::new("xcrun")
            .args(["notarytool", "history"])
            .args(notary_auth_args(auth))
            .output()
            .context("running `xcrun notarytool history`")?;
        if !status.status.success() {
            bail!(
                "the notary credentials were rejected: {}",
                String::from_utf8_lossy(&status.stderr).trim()
            );
        }
    }
    Ok(())
}

fn require_identity(listing: &str, identity: &str) -> anyhow::Result<()> {
    if identity_listed(listing, identity) {
        Ok(())
    } else {
        bail!(
            "signing identity '{identity}' was not found in the keychain; \
             `security find-identity -v` lists the usable identities"
        )
    }
}

/// Sign the `.app` with the hardened runtime, which notarization requires.
pub(super) fn codesign_app_command(plan: &SigningPlan, app_path: &str) -> Command {
    let mut cmd = Command::new("codesign");
    cmd.args(["--force", "--options", "runtime", "--timestamp"]);
    if let Some(entitlements) = &plan.entitlements {
        cmd.args(["--entitlements", entitlements]);
    }
    cmd.args(["--sign", &plan.identity]).arg(app_path);
    cmd
}

/// Sign a disk image.
pub(super) fn codesign_image_command(identity: &str, image: &str) -> Command {
    let mut cmd = Command::new("codesign");
    cmd.args(["--force", "--timestamp", "--sign", identity])
        .arg(image);
    cmd
}

/// Check the signature just made, so a broken bundle stops here and not at
/// Apple's notary service.
pub(super) fn codesign_verify_command(path: &str) -> Command {
    let mut cmd = Command::new("codesign");
    cmd.args(["--verify", "--strict", "--verbose=2"]).arg(path);
    cmd
}

/// Submit `artifact` to Apple and wait for the verdict.
pub(super) fn notarize_command(auth: &NotaryAuth, artifact: &str) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.args(["notarytool", "submit"])
        .arg(artifact)
        .arg("--wait")
        .args(notary_auth_args(auth));
    cmd
}

/// Attach the notarization ticket so the package opens offline.
pub(super) fn staple_command(artifact: &str) -> Command {
    let mut cmd = Command::new("xcrun");
    cmd.args(["stapler", "staple"]).arg(artifact);
    cmd
}

fn notary_auth_args(auth: &NotaryAuth) -> Vec<String> {
    match auth {
        NotaryAuth::KeychainProfile(profile) => {
            vec!["--keychain-profile".to_string(), profile.clone()]
        }
        NotaryAuth::ApiKey {
            path,
            key_id,
            issuer,
        } => vec![
            "--key".to_string(),
            path.clone(),
            "--key-id".to_string(),
            key_id.clone(),
            "--issuer".to_string(),
            issuer.clone(),
        ],
    }
}

#[cfg(test)]
mod tests;

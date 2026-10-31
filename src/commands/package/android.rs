//! Packaging an Android app into a signed `.aab` (for Google Play) or `.apk`
//! (for direct distribution).
//!
//! Gradle builds and signs the package. The project's own `build.gradle.kts`
//! signs release builds with the debug key, so `aimer` hands Gradle an init
//! script (`gradlew -I`) that swaps in the real keystore and the version from
//! `Aimer.toml` once the build script has run. Nothing in the scaffold has to
//! change, and existing projects work as they are.
//!
//! Settings come from `[android]` and `[android.signing]` in `Aimer.toml`; the
//! environment overrides them:
//!
//! | Variable                          | Overrides / provides                  |
//! |-----------------------------------|---------------------------------------|
//! | `AIMER_ANDROID_KEYSTORE`          | `keystore`                            |
//! | `AIMER_ANDROID_KEY_ALIAS`         | `key_alias`                           |
//! | `AIMER_ANDROID_KEYSTORE_PASSWORD` | the keystore password (env only)      |
//! | `AIMER_ANDROID_KEY_PASSWORD`      | the key password (env only; defaults to the keystore password) |
//! | `AIMER_ANDROID_VERSION_CODE`      | `version_code`                        |

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use super::sign::non_empty;
use super::{PackageFormat, Request, dist_dir, package_stem, reset_dir};
use crate::commands::assemble::{
    self, ANDROID_DEFAULT_ABI, AndroidPlan, Reporter, StdioReporter, Step, StepKind,
    android_project_dir, resolve_compatible_java_home,
};
use crate::config::{AimerManifest, AndroidMeta, resolve_cargo_package_name};

const ENV_KEYSTORE: &str = "AIMER_ANDROID_KEYSTORE";
const ENV_KEY_ALIAS: &str = "AIMER_ANDROID_KEY_ALIAS";
const ENV_KEYSTORE_PASSWORD: &str = "AIMER_ANDROID_KEYSTORE_PASSWORD";
const ENV_KEY_PASSWORD: &str = "AIMER_ANDROID_KEY_PASSWORD";
const ENV_VERSION_CODE: &str = "AIMER_ANDROID_VERSION_CODE";
const ENV_VERSION_NAME: &str = "AIMER_ANDROID_VERSION_NAME";

/// The init script that signs the build and sets its version.
const INIT_SCRIPT: &str = include_str!("../../../templates/android/aimer_package.init.gradle");
const INIT_SCRIPT_NAME: &str = "aimer_package.init.gradle";

/// Scratch space of one packaging run: the init script.
const WORK_DIR_NAME: &str = ".android-work";

/// Everything needed to sign and version one Android build.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct AndroidSigningPlan {
    /// The keystore, as an absolute path.
    pub keystore: String,
    pub key_alias: String,
    pub keystore_password: String,
    pub key_password: String,
    pub version_code: u32,
    /// The `versionName`; `None` keeps the one in the project.
    pub version_name: Option<String>,
}

// Not derived: the passwords must never end up in a log or a panic message.
impl fmt::Debug for AndroidSigningPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AndroidSigningPlan")
            .field("keystore", &self.keystore)
            .field("key_alias", &self.key_alias)
            .field("keystore_password", &"<redacted>")
            .field("key_password", &"<redacted>")
            .field("version_code", &self.version_code)
            .field("version_name", &self.version_name)
            .finish()
    }
}

impl AndroidSigningPlan {
    /// Combine the manifest settings with the environment.
    ///
    /// `env` is injected so the resolution can be tested without touching the
    /// process environment. `version` is `[package] version`.
    ///
    /// # Errors
    ///
    /// Fails, naming the setting and the variable that would provide it, when
    /// the keystore, the alias or the password is missing, or when no version
    /// code was given and none can be derived from `version`.
    pub(super) fn resolve(
        meta: Option<&AndroidMeta>,
        env: &dyn Fn(&str) -> Option<String>,
        version: &str,
        root: &Path,
    ) -> anyhow::Result<Self> {
        let signing = meta.and_then(|m| m.signing.as_ref());
        let keystore = non_empty(env(ENV_KEYSTORE))
            .or_else(|| non_empty(signing.and_then(|s| s.keystore.clone())));
        let Some(keystore) = keystore else {
            bail!(
                "no signing keystore: set `keystore` under [android.signing] in Aimer.toml \
                 or the {ENV_KEYSTORE} environment variable"
            );
        };
        let key_alias = non_empty(env(ENV_KEY_ALIAS))
            .or_else(|| non_empty(signing.and_then(|s| s.key_alias.clone())));
        let Some(key_alias) = key_alias else {
            bail!(
                "no key alias: set `key_alias` under [android.signing] in Aimer.toml \
                 or the {ENV_KEY_ALIAS} environment variable"
            );
        };
        let Some(keystore_password) = non_empty(env(ENV_KEYSTORE_PASSWORD)) else {
            bail!("no keystore password: set the {ENV_KEYSTORE_PASSWORD} environment variable");
        };
        let key_password =
            non_empty(env(ENV_KEY_PASSWORD)).unwrap_or_else(|| keystore_password.clone());

        let explicit = match non_empty(env(ENV_VERSION_CODE)) {
            Some(text) => Some(
                text.parse::<u32>()
                    .with_context(|| format!("{ENV_VERSION_CODE} must be a whole number"))?,
            ),
            None => meta.and_then(|m| m.version_code),
        };
        let version = version.trim();

        Ok(Self {
            keystore: root.join(&keystore).to_string_lossy().into_owned(),
            key_alias,
            keystore_password,
            key_password,
            version_code: version_code(explicit, version)?,
            version_name: (!version.is_empty()).then(|| version.to_string()),
        })
    }

    /// Check what can be checked before the (slow) build: the keystore exists,
    /// opens with the password and holds the alias.
    pub(super) fn preflight(&self, java_home: Option<&str>) -> anyhow::Result<()> {
        if !Path::new(&self.keystore).is_file() {
            bail!("the keystore '{}' does not exist", self.keystore);
        }
        let output = self.keytool_command(java_home).output();
        match output {
            Ok(output) if !output.status.success() => bail!(
                "the keystore '{}' could not be opened with alias '{}': {}",
                self.keystore,
                self.key_alias,
                String::from_utf8_lossy(&output.stdout).trim()
            ),
            // Without keytool there is nothing to check with; Gradle will say
            // if the keystore is wrong.
            _ => Ok(()),
        }
    }

    /// `keytool -list` of the alias; the password travels in the environment,
    /// not on the command line where `ps` would show it.
    fn keytool_command(&self, java_home: Option<&str>) -> Command {
        let keytool = match java_home {
            Some(home) => Path::new(home).join("bin/keytool"),
            None => PathBuf::from("keytool"),
        };
        let mut cmd = Command::new(keytool);
        cmd.args([
            "-list",
            "-keystore",
            &self.keystore,
            "-alias",
            &self.key_alias,
        ])
        .args(["-storepass:env", ENV_KEYSTORE_PASSWORD])
        .env(ENV_KEYSTORE_PASSWORD, &self.keystore_password);
        cmd
    }

    /// The environment the init script reads.
    fn gradle_env(&self) -> Vec<(&'static str, String)> {
        let mut env = vec![
            (ENV_KEYSTORE, self.keystore.clone()),
            (ENV_KEY_ALIAS, self.key_alias.clone()),
            (ENV_KEYSTORE_PASSWORD, self.keystore_password.clone()),
            (ENV_KEY_PASSWORD, self.key_password.clone()),
            (ENV_VERSION_CODE, self.version_code.to_string()),
        ];
        if let Some(name) = &self.version_name {
            env.push((ENV_VERSION_NAME, name.clone()));
        }
        env
    }
}

/// The `versionCode`: the explicit one, or `major * 1_000_000 + minor * 1_000 +
/// patch` of `version`, which grows with every release of a sane version
/// number.
///
/// # Errors
///
/// Fails when there is no explicit code and `version` is empty, is not made of
/// numbers, or has a part above 999.
pub(super) fn version_code(explicit: Option<u32>, version: &str) -> anyhow::Result<u32> {
    if let Some(code) = explicit {
        return Ok(code);
    }
    let hint = "set `version_code` under [android] in Aimer.toml or the \
                AIMER_ANDROID_VERSION_CODE environment variable";
    if version.is_empty() {
        bail!(
            "no version to derive the Android versionCode from: set `version` in Aimer.toml, or {hint}"
        );
    }
    // `1.2.3-beta+4` is 1.2.3: the suffix does not order releases.
    let numbers = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts = [0u32; 3];
    for (index, part) in numbers.split('.').enumerate() {
        let Some(slot) = parts.get_mut(index) else {
            bail!("version '{version}' has more than three parts; {hint}");
        };
        *slot = part.parse().ok().filter(|n| *n <= 999).with_context(|| {
            format!("version '{version}' is not made of numbers up to 999; {hint}")
        })?;
    }
    Ok(parts[0] * 1_000_000 + parts[1] * 1_000 + parts[2])
}

/// What to build and where the results go.
pub(super) struct ArtifactJob<'a> {
    pub format: PackageFormat,
    /// The application name, which names the package.
    pub app_name: &'a str,
    pub version: &'a str,
    /// The Android Gradle project (`builds/android`).
    pub project_dir: &'a Path,
    /// Directory the package is written to.
    pub dist: &'a str,
    /// The JDK Gradle should run on, if one was found.
    pub java_home: Option<&'a str>,
}

/// Build the Android app and package it as the request says; returns the
/// package.
pub(super) fn run(request: &Request, manifest: Option<&AimerManifest>) -> anyhow::Result<String> {
    let version = manifest
        .map(|m| m.package.version.clone())
        .unwrap_or_default();
    let root = std::env::current_dir().context("reading the current directory")?;
    // Checked before the (slow) build, so a missing keystore or a wrong
    // password fails in seconds.
    let plan = AndroidSigningPlan::resolve(
        manifest.and_then(|m| m.android.as_ref()),
        &|key| std::env::var(key).ok(),
        &version,
        &root,
    )?;
    let java_home = resolve_compatible_java_home().or_else(|| std::env::var("JAVA_HOME").ok());
    plan.preflight(java_home.as_deref())?;

    println!(
        "Packaging 'android' app into a signed .{} (versionCode {}, {})...",
        request.format.extension(),
        plan.version_code,
        plan.version_name.as_deref().unwrap_or("version unchanged"),
    );

    let reporter = StdioReporter;
    let pkg_name = resolve_cargo_package_name(Path::new("."));
    let android = AndroidPlan::for_abi(ANDROID_DEFAULT_ABI);
    assemble::build_android(&android, true, &reporter)?;
    assemble::stage_android_inputs(&pkg_name, &android, true, &reporter)?;

    let app_name = assemble::app_name();
    let dist = dist_dir(true);
    let project_dir = android_project_dir();
    let job = ArtifactJob {
        format: request.format,
        app_name: &app_name,
        version: &version,
        project_dir: &project_dir,
        dist: &dist,
        java_home: java_home.as_deref(),
    };
    package_artifact(&job, &plan, &reporter)
}

/// Run Gradle with the init script and copy its output into the dist folder;
/// returns the path of the package.
pub(super) fn package_artifact(
    job: &ArtifactJob,
    plan: &AndroidSigningPlan,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    std::fs::create_dir_all(job.dist).with_context(|| format!("creating {}", job.dist))?;
    let work = std::path::absolute(format!("{}/{WORK_DIR_NAME}", job.dist))
        .context("resolving the scratch directory")?;
    reset_dir(&work.to_string_lossy())?;
    let result = build_and_copy(job, plan, &work, reporter);
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn build_and_copy(
    job: &ArtifactJob,
    plan: &AndroidSigningPlan,
    work: &Path,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    let init_script = work.join(INIT_SCRIPT_NAME);
    std::fs::write(&init_script, INIT_SCRIPT)
        .with_context(|| format!("writing {}", init_script.display()))?;

    // A package from an earlier run must never be mistaken for this one's.
    let built = gradle_output(job.project_dir, job.format);
    match std::fs::remove_file(&built) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing {}", built.display())),
    }

    reporter.note(format!(
        "Building the signed .{} via Gradle...",
        job.format.extension()
    ));
    if let Some(home) = job.java_home {
        reporter.note(format!("Using JAVA_HOME: {home}"));
    }
    reporter.run(
        gradle_command(job, plan, &init_script),
        Step::new(StepKind::Gradle, "Gradle package for Android"),
    )?;
    if !built.is_file() {
        bail!("Gradle finished but {} was not produced", built.display());
    }

    let output = format!(
        "{}/{}.{}",
        job.dist,
        package_stem(job.app_name, job.version),
        job.format.extension()
    );
    match std::fs::remove_file(&output) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing stale package {output}")),
    }
    std::fs::copy(&built, &output).with_context(|| format!("copying {}", built.display()))?;
    Ok(output)
}

/// Where Gradle leaves the release package of `format`.
pub(super) fn gradle_output(project_dir: &Path, format: PackageFormat) -> PathBuf {
    match format {
        PackageFormat::Aab => project_dir.join("app/build/outputs/bundle/release/app-release.aab"),
        _ => project_dir.join("app/build/outputs/apk/release/app-release.apk"),
    }
}

/// The Gradle task that builds the release package of `format`.
pub(super) fn gradle_task(format: PackageFormat) -> &'static str {
    match format {
        PackageFormat::Aab => "bundleRelease",
        _ => "assembleRelease",
    }
}

/// The Gradle wrapper, run with the init script and the signing environment.
pub(super) fn gradle_command(
    job: &ArtifactJob,
    plan: &AndroidSigningPlan,
    init_script: &Path,
) -> Command {
    let gradlew = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    };
    let mut cmd = Command::new(job.project_dir.join(gradlew));
    cmd.arg("-I")
        .arg(init_script)
        .arg(gradle_task(job.format))
        .current_dir(job.project_dir)
        .envs(plan.gradle_env());
    if let Some(home) = job.java_home {
        cmd.env("JAVA_HOME", home);
    }
    cmd
}

#[cfg(test)]
mod tests;

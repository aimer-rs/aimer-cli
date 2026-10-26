pub(crate) mod link_flags;

use std::env::current_dir;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};

use crate::commands::run::helpers::host_arch;
use crate::commands::run::utilities::get_project_root;
use crate::config::{resolve_cargo_package_name, resolve_package_name};
use crate::errors::AimerError;
use crate::targets::Targets;

/// The Rust target the macOS app is compiled for.
pub(crate) const MACOS_RUST_TARGET: &str = "aarch64-apple-darwin";

/// ABI assumed when no device is involved, i.e. for `aimer assemble android`.
pub(crate) const ANDROID_DEFAULT_ABI: &str = "arm64-v8a";

/// The generated Xcode project of the iOS app, relative to the project root.
pub(crate) const IOS_PROJECT_DIR: &str = "builds/ios";
/// The Xcode project directory of the macOS app.
pub(crate) const MACOS_PROJECT_DIR: &str = "builds/macos";

/// Non-interactive bundling entry point used by `aimer assemble <platform>`.
///
/// Unlike `aimer build` (which only compiles the Rust library) this command
/// produces the *distributable platform bundle* — a `.app` on macOS/iOS, an
/// `.apk` on Android, or the static web `dist/` directory — in either debug or
/// release mode.
///
/// The packaging steps themselves ([`package_macos`], [`package_ios`],
/// [`package_android`]) are shared with the interactive `aimer run` pipeline;
/// the only difference is the [`Reporter`] they are handed. Here that is
/// [`StdioReporter`], which runs each step synchronously with inherited stdio so
/// it is friendly to CI logs.
pub fn execute(platform: String, release: bool) -> anyhow::Result<()> {
    execute_with_package(platform, release, None)
}

pub fn execute_with_package(
    platform: String,
    release: bool,
    package: Option<String>,
) -> anyhow::Result<()> {
    // Validate the requested platform before touching project selection, so an
    // unknown or non-assemblable platform is reported on its own terms rather
    // than being masked by an unrelated "no Aimer project found" error.
    let target = Targets::try_from(platform.as_str())
        .map_err(|_| AimerError::UnknownTarget(platform.clone()))?;
    if target == Targets::Terminated {
        bail!("'terminated' is not an assemblable platform");
    }

    let project = crate::commands::project_selection::select_aimer_project(package.as_deref())?;
    let _project_directory = project.enter()?;

    println!(
        "Assembling '{target}' bundle in {} mode...",
        profile_name(release)
    );

    // Cargo names the binary and library after `Cargo.toml`; the manifest's
    // application name only decides what the bundle is called.
    let pkg_name = resolve_cargo_package_name(Path::new("."));
    let reporter = StdioReporter;

    let artifact = match target {
        Targets::Macos => {
            build_macos(release, &reporter)?;
            package_macos(&pkg_name, release, &reporter)?
        }
        Targets::Ios | Targets::IosSimulator => {
            let plan = IosPlan::resolve(target == Targets::IosSimulator);
            build_ios(&plan, release, &reporter)?;
            package_ios(&pkg_name, &plan, release, &reporter)?
        }
        Targets::Android | Targets::AndroidSimulator => {
            let plan = AndroidPlan::for_abi(ANDROID_DEFAULT_ABI);
            build_android(&plan, release, &reporter)?;
            package_android(&pkg_name, &plan, release, &reporter)?
        }
        Targets::Web => assemble_web(release, &reporter)?,
        Targets::Windows | Targets::Linux => {
            build_desktop(target, &pkg_name, release, &reporter)?;
            package_desktop(target, &pkg_name, release, &reporter)?
        }
        Targets::Terminated => bail!("'terminated' is not an assemblable platform"),
    };

    println!("Bundle assembled successfully: {artifact}");
    Ok(())
}

/// The tool an assemble step runs, so a [`Reporter`] can pick the right
/// progress parsing for its output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepKind {
    /// `cargo` or `cargo ndk`.
    Cargo,
    /// `xcodebuild`.
    Xcode,
    /// The Gradle wrapper.
    Gradle,
    /// Anything else, e.g. `trunk`.
    Other,
}

/// One command an assemble stage runs, together with the human readable action
/// used in its progress and error messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub kind: StepKind,
    pub action: String,
}

impl Step {
    /// A step running `kind` to carry out `action`.
    #[inline]
    pub(crate) fn new(kind: StepKind, action: impl Into<String>) -> Self {
        Self {
            kind,
            action: action.into(),
        }
    }
}

/// Where an assemble step sends its output.
///
/// The same packaging code serves `aimer assemble`, which prints synchronously
/// to stdout ([`StdioReporter`]), and `aimer run`, which streams every line into
/// the TUI console
/// ([`ConsoleReporter`](crate::commands::run::helpers::ConsoleReporter)).
pub(crate) trait Reporter {
    /// Report one progress line.
    fn note(&self, message: String);

    /// Run `cmd` to completion, reporting its output, and fail when it cannot be
    /// started or exits with a non-zero status.
    fn run(&self, cmd: Command, step: Step) -> anyhow::Result<()>;

    /// Stage every file registered under `[assets]` in `Aimer.toml` into
    /// `dest_root`.
    ///
    /// Copying is incremental — only new or changed files are written — so a
    /// rebuild does not re-copy unchanged assets. A registered file that does
    /// not exist is reported as a warning and skipped; only a broken asset
    /// registry is an error.
    fn stage_assets(&self, dest_root: &str) -> anyhow::Result<()> {
        self.note(format!("Copying assets into {dest_root}"));
        let report = crate::commands::assets::copy_assets_into(dest_root)?;
        for rel in &report.copied {
            self.note(format!(
                "Copied asset {rel} -> {}",
                Path::new(dest_root).join(rel).display()
            ));
        }
        for rel in &report.missing {
            self.note(format!(
                "warning: registered asset '{rel}' not found; skipping"
            ));
        }
        Ok(())
    }
}

/// [`Reporter`] for the non-interactive `aimer assemble`: every step inherits
/// this process' stdio and progress goes to stdout.
pub(crate) struct StdioReporter;

impl Reporter for StdioReporter {
    fn note(&self, message: String) {
        println!("{message}");
    }

    fn run(&self, mut cmd: Command, step: Step) -> anyhow::Result<()> {
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        let status = cmd
            .status()
            .with_context(|| format!("failed to start {}", step.action))?;
        if !status.success() {
            bail!("{} failed", step.action);
        }
        Ok(())
    }
}

/// The Cargo profile directory name for the requested build mode.
pub(crate) fn profile_name(release: bool) -> &'static str {
    if release { "release" } else { "debug" }
}

/// The Xcode `-configuration` value for the requested build mode.
pub(crate) fn xcode_configuration(release: bool) -> &'static str {
    if release { "Release" } else { "Debug" }
}

/// The Gradle assemble task for the requested build mode.
pub(crate) fn gradle_task(release: bool) -> &'static str {
    if release {
        "assembleRelease"
    } else {
        "assembleDebug"
    }
}

/// Absolute path of the compiled Rust artifact for `rust_target`/`profile`.
pub(crate) fn artifact_path(
    rust_target: &str,
    lib_name: &str,
    release: bool,
    extension: &str,
) -> String {
    let root = get_project_root(true).unwrap_or_else(|_| current_dir().unwrap());
    format!(
        "{}/target/{}/{}/lib{}{}",
        root.display(),
        rust_target,
        profile_name(release),
        lib_name,
        extension
    )
}

/// The library name cargo derives from a package name.
pub(crate) fn lib_name_of(pkg_name: &str) -> String {
    pkg_name.replace('-', "_")
}

/// Copy a freshly compiled native library into `dest_dir`, creating the
/// directory tree first.
fn copy_lib(
    src: &str,
    dest_dir: &str,
    lib_file: &str,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest_dir).with_context(|| format!("creating {dest_dir}"))?;
    let dest = format!("{dest_dir}/{lib_file}");
    std::fs::copy(src, &dest)
        .with_context(|| format!("copying native library '{src}' -> '{dest}'"))?;
    reporter.note(format!("Copied library to {dest}"));
    Ok(())
}

/// An `xcodebuild` invocation for the project in `project_dir`.
fn xcode_command(
    project_dir: &str,
    pkg_name: &str,
    app_name: &str,
    configuration: &str,
    sdk: Option<&str>,
    arch: &str,
    force_load_library: Option<&str>,
) -> Command {
    let mut cmd = Command::new("xcodebuild");
    cmd.arg("-project")
        .arg(format!("{pkg_name}.xcodeproj"))
        .arg("-target")
        .arg(pkg_name)
        .arg("-configuration")
        .arg(configuration);
    if let Some(sdk) = sdk {
        cmd.arg("-sdk").arg(sdk);
    }
    if let Some(library) = force_load_library {
        cmd.arg(format!("OTHER_LDFLAGS=$(inherited) -force_load {library}"));
    }
    // The scaffold names the product after its target, which is the Cargo
    // package; the override makes the bundle carry the application name.
    cmd.arg(format!("PRODUCT_NAME={app_name}"))
        .arg("SYMROOT=build")
        .arg("-arch")
        .arg(arch)
        .current_dir(project_dir);
    cmd
}

/// The application name the bundle is shipped under, read from `Aimer.toml` in
/// the current project directory.
#[inline]
pub(crate) fn app_name() -> String {
    resolve_package_name(Path::new("."))
}

/// The `.app` bundle `xcodebuild` produces for a macOS package named
/// `app_name`.
pub(crate) fn macos_app_path(app_name: &str, release: bool) -> String {
    format!(
        "builds/macos/build/{}/{}.app",
        xcode_configuration(release),
        app_name
    )
}

/// The executable inside a macOS `.app` bundle, which Xcode names after the
/// product.
pub(crate) fn macos_executable_path(app_path: &str, app_name: &str) -> String {
    format!("{app_path}/Contents/MacOS/{app_name}")
}

/// Fail unless `target` is the desktop platform of the host.
///
/// The desktop leg is a host build: cargo compiles for the default triple and
/// the bundle is laid out for the operating system it was produced on. Asking
/// for the other desktop platform can therefore only ever yield an artifact
/// that cannot run, so it is rejected up front instead.
///
/// # Errors
///
/// Returns an error naming both the requested target and the host when they do
/// not match, or when the host is not a desktop platform at all.
pub(crate) fn ensure_host_desktop(target: Targets) -> anyhow::Result<()> {
    match Targets::host_desktop() {
        Some(host) if host == target => Ok(()),
        _ => bail!(
            "cross-compiling to {target} from {} is not supported; \
             build desktop targets on a matching host",
            std::env::consts::OS
        ),
    }
}

/// The portable desktop bundle of `pkg_name`, relative to the project root.
///
/// The bundle is a plain folder — executable, staged assets and the platform
/// scaffold files side by side — so it can be zipped and handed over as is.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(
///     desktop_bundle_path(Targets::Linux, "my_app", false),
///     "builds/linux/bundle/debug/my_app"
/// );
/// ```
pub(crate) fn desktop_bundle_path(target: Targets, pkg_name: &str, release: bool) -> String {
    format!(
        "builds/{target}/bundle/{}/{pkg_name}",
        profile_name(release)
    )
}

/// The executable cargo produces for a host build of `pkg_name`.
///
/// Cargo names the default binary after the package, dashes included, so the
/// raw package name is used here rather than [`lib_name_of`].
pub(crate) fn desktop_exe_path(pkg_name: &str, release: bool) -> String {
    let project_path = get_project_root(true)
        .unwrap_or_default()
        .display()
        .to_string();
    format!(
        "{project_path}/target/{}/{pkg_name}{}",
        profile_name(release),
        std::env::consts::EXE_SUFFIX
    )
}

/// The file name the executable carries inside the bundle.
#[inline]
pub(crate) fn desktop_exe_name(pkg_name: &str) -> String {
    format!("{pkg_name}{}", std::env::consts::EXE_SUFFIX)
}

/// Remove `path` if it exists, so a stale executable or asset can never be
/// launched when the packager decides it has nothing to do.
fn clean_bundle(path: &str) -> anyhow::Result<()> {
    let bundle = Path::new(path);
    if bundle.exists() {
        std::fs::remove_dir_all(bundle).with_context(|| format!("cleaning {path}"))?;
    }
    Ok(())
}

/// Compile the Rust library for macOS.
///
/// Like [`build_ios`] this goes through `cargo rustc`, so the compiler writes
/// down the system frameworks the crate graph links against for
/// [`package_macos`] to hand to Xcode. See [`link_flags`].
fn build_macos(release: bool, reporter: &dyn Reporter) -> anyhow::Result<()> {
    let mut cargo = Command::new("cargo");
    cargo
        .arg("rustc")
        .args(["--target", MACOS_RUST_TARGET, "--lib"]);
    if release {
        cargo.arg("--release");
    }
    cargo
        .arg("--")
        .arg("--print")
        .arg(link_flags::print_arg(MACOS_RUST_TARGET, release)?);
    reporter.run(cargo, Step::new(StepKind::Cargo, "cargo build for macOS"))
}

/// Turn the compiled macOS library into a launchable `.app` bundle and return
/// its path.
///
/// Shared by `aimer assemble macos` and the macOS leg of the `aimer run`
/// pipeline: the static archive is staged into the Xcode project, the bundle is
/// rebuilt from scratch and the registered assets land in `Contents/Resources`.
pub(crate) fn package_macos(
    pkg_name: &str,
    release: bool,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    package_macos_with_options(pkg_name, release, reporter, false)
}

/// Package a macOS hot-reload host with the Rust archive force-loaded.
///
/// The force-load is scoped to this packaging path because portable widget
/// registrations are linker-only symbols. Normal macOS builds retain their
/// existing linker behavior.
pub(crate) fn package_macos_for_hot_reload(
    pkg_name: &str,
    release: bool,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    package_macos_with_options(pkg_name, release, reporter, true)
}

fn package_macos_with_options(
    pkg_name: &str,
    release: bool,
    reporter: &dyn Reporter,
    force_load_rust_library: bool,
) -> anyhow::Result<String> {
    let lib_name = lib_name_of(pkg_name);
    let src_lib = artifact_path(MACOS_RUST_TARGET, &lib_name, release, ".a");
    copy_lib(
        &src_lib,
        "builds/macos/Libraries",
        &format!("lib{lib_name}.a"),
        reporter,
    )?;

    // The archive alone does not tell Xcode which frameworks to link, so the
    // list the compiler last wrote is turned into the project's xcconfig.
    link_flags::refresh(
        Path::new(MACOS_PROJECT_DIR),
        &link_flags::raw_path(MACOS_RUST_TARGET, release),
    )?;

    let app_name = app_name();
    let artifact = macos_app_path(&app_name, release);
    clean_bundle(&artifact)?;

    let arch = host_arch();
    let force_load_library =
        force_load_rust_library.then(|| format!("$(SRCROOT)/Libraries/lib{lib_name}.a"));
    reporter.note(format!("Building Xcode project for {arch}..."));
    reporter.run(
        xcode_command(
            MACOS_PROJECT_DIR,
            pkg_name,
            &app_name,
            xcode_configuration(release),
            None,
            arch,
            force_load_library.as_deref(),
        ),
        Step::new(StepKind::Xcode, "xcodebuild for macOS"),
    )?;

    reporter.stage_assets(&format!("{artifact}/Contents/Resources"))?;
    Ok(artifact)
}

/// Everything that differs between an iOS device build and a simulator build,
/// resolved once from the flavour and the host architecture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IosPlan {
    /// Whether the bundle targets the simulator rather than a device.
    pub simulator: bool,
    /// Rust target triple the static library is compiled for.
    pub rust_target: &'static str,
    /// `xcodebuild -sdk` value, which also names the build subdirectory.
    pub sdk: &'static str,
    /// `xcodebuild -arch` value.
    pub arch: &'static str,
}

impl IosPlan {
    /// Resolve the plan for a device (`simulator = false`) or the simulator on
    /// this host.
    pub(crate) fn resolve(simulator: bool) -> Self {
        let arch = host_arch();
        if simulator {
            Self {
                simulator,
                rust_target: if arch == "x86_64" {
                    "x86_64-apple-ios"
                } else {
                    "aarch64-apple-ios-sim"
                },
                sdk: "iphonesimulator",
                arch,
            }
        } else {
            Self {
                simulator,
                rust_target: "aarch64-apple-ios",
                sdk: "iphoneos",
                arch,
            }
        }
    }

    /// The `.app` bundle `xcodebuild` produces for the application named
    /// `app_name`.
    pub(crate) fn app_path(&self, app_name: &str, release: bool) -> String {
        format!(
            "builds/ios/build/{}-{}/{}.app",
            xcode_configuration(release),
            self.sdk,
            app_name
        )
    }
}

/// Compile the Rust library for iOS.
///
/// This goes through `cargo rustc` rather than `cargo build` for one reason:
/// the trailing `--print native-static-libs` makes the compiler write down
/// which system frameworks the crate graph links against, which
/// [`package_ios`] then feeds to Xcode. See [`link_flags`].
fn build_ios(plan: &IosPlan, release: bool, reporter: &dyn Reporter) -> anyhow::Result<()> {
    let mut cargo = Command::new("cargo");
    cargo
        .arg("rustc")
        .arg("--lib")
        .arg("--target")
        .arg(plan.rust_target);
    if release {
        cargo.arg("--release");
    }
    cargo
        .arg("--")
        .arg("--print")
        .arg(link_flags::print_arg(plan.rust_target, release)?);
    reporter.run(cargo, Step::new(StepKind::Cargo, "cargo build for iOS"))
}

/// Turn the compiled iOS library into a launchable `.app` bundle and return its
/// path.
///
/// Shared by `aimer assemble ios` and the iOS leg of the `aimer run` pipeline.
pub(crate) fn package_ios(
    pkg_name: &str,
    plan: &IosPlan,
    release: bool,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    let app_name = app_name();
    let artifact = plan.app_path(&app_name, release);
    clean_bundle(&artifact)?;

    let lib_name = lib_name_of(pkg_name);
    let src_lib = artifact_path(plan.rust_target, &lib_name, release, ".a");
    copy_lib(
        &src_lib,
        "builds/ios/Libraries",
        &format!("lib{lib_name}.a"),
        reporter,
    )?;

    // The archive alone does not tell Xcode which frameworks to link, so the
    // list the compiler just wrote is turned into the project's xcconfig.
    link_flags::refresh(
        Path::new(IOS_PROJECT_DIR),
        &link_flags::raw_path(plan.rust_target, release),
    )?;

    reporter.note("Building Xcode project for iOS...".to_string());
    reporter.run(
        xcode_command(
            IOS_PROJECT_DIR,
            pkg_name,
            &app_name,
            xcode_configuration(release),
            Some(plan.sdk),
            plan.arch,
            None,
        ),
        Step::new(StepKind::Xcode, "xcodebuild for iOS"),
    )?;

    reporter.stage_assets(&artifact)?;
    Ok(artifact)
}

/// The Rust target and JNI directory matching an Android ABI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AndroidPlan {
    pub rust_target: &'static str,
    pub jni_dir: &'static str,
}

impl AndroidPlan {
    /// Map the `ro.product.cpu.abi` property of a device to a build plan,
    /// defaulting to 64-bit ARM for an unknown ABI.
    pub(crate) fn for_abi(abi: &str) -> Self {
        let (rust_target, jni_dir) = match abi {
            "x86_64" => ("x86_64-linux-android", "x86_64"),
            "armeabi-v7a" => ("armv7-linux-androideabi", "armeabi-v7a"),
            "x86" => ("i686-linux-android", "x86"),
            _ => ("aarch64-linux-android", "arm64-v8a"),
        };
        Self {
            rust_target,
            jni_dir,
        }
    }

    /// Where Gradle expects the shared library for this ABI.
    pub(crate) fn jni_libs_dir(&self) -> String {
        format!("builds/android/app/src/main/jniLibs/{}", self.jni_dir)
    }
}

/// Absolute path of the generated Android project.
pub(crate) fn android_project_dir() -> PathBuf {
    current_dir().unwrap_or_default().join("builds/android")
}

/// A Gradle wrapper invocation assembling the APK for the requested mode.
fn gradle_command(project_dir: &Path, release: bool) -> Command {
    let gradlew = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    };
    let mut cmd = Command::new(project_dir.join(gradlew));
    cmd.arg(gradle_task(release)).current_dir(project_dir);
    cmd
}

/// The APKs Gradle may have produced for `release`, most preferred first.
///
/// Unsigned release builds (no signing config) are emitted with an `-unsigned`
/// suffix, so both names are candidates.
pub(crate) fn apk_candidates(release: bool) -> Vec<String> {
    if release {
        vec![
            "builds/android/app/build/outputs/apk/release/app-release.apk".to_string(),
            "builds/android/app/build/outputs/apk/release/app-release-unsigned.apk".to_string(),
        ]
    } else {
        vec!["builds/android/app/build/outputs/apk/debug/app-debug.apk".to_string()]
    }
}

/// The APK Gradle produced, or the output directory when none of the known
/// names exists.
pub(crate) fn apk_path(release: bool) -> String {
    apk_candidates(release)
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or_else(|| {
            format!(
                "builds/android/app/build/outputs/apk/{}/",
                profile_name(release)
            )
        })
}

/// Compile the Rust library for Android through `cargo ndk`.
fn build_android(plan: &AndroidPlan, release: bool, reporter: &dyn Reporter) -> anyhow::Result<()> {
    let mut cargo = Command::new("cargo");
    cargo
        .arg("ndk")
        .arg("-t")
        .arg(plan.jni_dir)
        .arg("build")
        .arg("--lib");
    if release {
        cargo.arg("--release");
    }
    reporter.run(
        cargo,
        Step::new(StepKind::Cargo, "cargo ndk build for Android"),
    )
}

/// Turn the compiled Android library into an installable APK and return its
/// path.
///
/// Shared by `aimer assemble android` and the Android leg of the `aimer run`
/// pipeline: the shared object is staged into `jniLibs`, the registered assets
/// into the APK's `assets/` source set, then Gradle packs it.
pub(crate) fn package_android(
    pkg_name: &str,
    plan: &AndroidPlan,
    release: bool,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    let lib_name = lib_name_of(pkg_name);
    let src_lib = artifact_path(plan.rust_target, &lib_name, release, ".so");
    copy_lib(
        &src_lib,
        &plan.jni_libs_dir(),
        &format!("lib{lib_name}.so"),
        reporter,
    )?;

    // Stage assets into the APK's `assets/` source set before Gradle packs it,
    // so they are readable at runtime via AssetManager.
    reporter.stage_assets("builds/android/app/src/main/assets")?;

    let project_dir = android_project_dir();
    reporter.note("Building Android project via Gradle...".to_string());

    let mut gradle = gradle_command(&project_dir, release);
    if let Some(java_home) = resolve_compatible_java_home() {
        reporter.note(format!("Using JAVA_HOME: {java_home}"));
        gradle.env("JAVA_HOME", java_home);
    }
    reporter.run(
        gradle,
        Step::new(StepKind::Gradle, "Gradle assemble for Android"),
    )?;

    Ok(apk_path(release))
}

/// Build the static web bundle via `trunk`.
fn assemble_web(release: bool, reporter: &dyn Reporter) -> anyhow::Result<String> {
    // Trunk cleans dist/ before building, so the registered assets are staged
    // next to the sources it copies from instead of into dist/ directly.
    reporter.stage_assets("builds/web")?;

    let llvm_ar = crate::commands::run::web::find_llvm_ar().context("Failed to find llvm-ar")?;
    let mut trunk = Command::new("trunk");
    crate::commands::run::web::configure_trunk(&mut trunk, &llvm_ar);
    trunk.arg("build").current_dir("builds/web");
    if release {
        trunk.arg("--release");
    }
    reporter.run(trunk, Step::new(StepKind::Other, "trunk build for web"))?;

    Ok("builds/web/dist".to_string())
}

/// Compile the application binary for the host desktop.
///
/// Unlike every other platform the desktop leg builds a `--bin` rather than a
/// `--lib`: there is no platform shell to link the library into, the compiled
/// executable *is* the app.
pub(crate) fn build_desktop(
    target: Targets,
    pkg_name: &str,
    release: bool,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    ensure_host_desktop(target)?;
    let mut cargo = Command::new("cargo");
    cargo.arg("build").args(["--bin", pkg_name]);
    if release {
        cargo.arg("--release");
    }
    reporter.run(
        cargo,
        Step::new(StepKind::Cargo, format!("cargo build for {target}")),
    )
}

/// Stage the executable, the registered assets and the platform scaffold files
/// into the portable bundle, returning its path.
///
/// Shared by `aimer assemble windows|linux` and the desktop leg of the
/// `aimer run` pipeline. The bundle is rebuilt from scratch every time, so a
/// stale executable can never be launched after a failed build.
pub(crate) fn package_desktop(
    target: Targets,
    pkg_name: &str,
    release: bool,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    ensure_host_desktop(target)?;
    package_desktop_in(Path::new("."), target, pkg_name, release, reporter)
}

/// [`package_desktop`], rooted at `root` instead of the current directory and
/// without the host check.
///
/// The returned bundle path stays relative to `root`, which is what the console
/// and the assemble summary print. Only the layout is decided here, so the
/// staging can be exercised for either desktop platform from any host.
pub(crate) fn package_desktop_in(
    root: &Path,
    target: Targets,
    pkg_name: &str,
    release: bool,
    reporter: &dyn Reporter,
) -> anyhow::Result<String> {
    let bundle = desktop_bundle_path(target, pkg_name, release);
    let bundle_dir = root.join(&bundle);
    clean_bundle(&bundle_dir.to_string_lossy())?;
    std::fs::create_dir_all(&bundle_dir)
        .with_context(|| format!("creating {}", bundle_dir.display()))?;

    let src = root.join(desktop_exe_path(pkg_name, release));
    let dest = bundle_dir.join(desktop_exe_name(pkg_name));
    // `fs::copy` carries the permission bits over, so the copy stays executable.
    std::fs::copy(&src, &dest).with_context(|| {
        format!(
            "copying executable '{}' -> '{}'",
            src.display(),
            dest.display()
        )
    })?;
    reporter.note(format!("Copied executable to {}", dest.display()));

    copy_desktop_extras(root, target, pkg_name, &bundle_dir, reporter)?;

    // The runtime looks assets up next to the executable, so the bundle root is
    // exactly where they have to land.
    reporter.stage_assets(&bundle)?;
    Ok(bundle)
}

/// Copy the platform scaffold files of `target` into the bundle.
///
/// Linux gets its icon and a desktop entry whose `Exec`/`Icon` lines name the
/// bundled binary; Windows gets its icon and the manifest under the
/// `<exe>.manifest` name the loader picks up as an external manifest. A scaffold
/// file that was never generated is reported and skipped rather than failing the
/// bundle — the executable alone is still runnable.
fn copy_desktop_extras(
    root: &Path,
    target: Targets,
    pkg_name: &str,
    bundle_dir: &Path,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    let scaffold = root.join(format!("builds/{target}"));
    match target {
        Targets::Linux => {
            copy_extra(
                &scaffold.join("app.png"),
                &bundle_dir.join("app.png"),
                reporter,
            )?;
            let entry = [
                scaffold.join(format!("{pkg_name}.desktop")),
                scaffold.join("app.desktop"),
            ]
            .into_iter()
            .find(|p| p.exists());
            if let Some(entry) = entry {
                let contents = std::fs::read_to_string(&entry)
                    .with_context(|| format!("reading {}", entry.display()))?;
                let dest = bundle_dir.join(format!("{pkg_name}.desktop"));
                std::fs::write(&dest, retarget_desktop_entry(&contents, pkg_name))
                    .with_context(|| format!("writing {}", dest.display()))?;
                reporter.note(format!("Copied desktop entry to {}", dest.display()));
            } else {
                reporter.note(format!(
                    "warning: no desktop entry in {}; skipping",
                    scaffold.display()
                ));
            }
        }
        Targets::Windows => {
            copy_extra(
                &scaffold.join("app.ico"),
                &bundle_dir.join("app.ico"),
                reporter,
            )?;
            copy_extra(
                &scaffold.join("app.manifest"),
                &bundle_dir.join(format!("{}.manifest", desktop_exe_name(pkg_name))),
                reporter,
            )?;
        }
        _ => {}
    }
    Ok(())
}

/// Copy one optional scaffold file, reporting a missing source instead of
/// failing.
fn copy_extra(src: &Path, dest: &Path, reporter: &dyn Reporter) -> anyhow::Result<()> {
    if !src.exists() {
        reporter.note(format!(
            "warning: scaffold file '{}' not found; skipping",
            src.display()
        ));
        return Ok(());
    }
    std::fs::copy(src, dest)
        .with_context(|| format!("copying '{}' -> '{}'", src.display(), dest.display()))?;
    reporter.note(format!("Copied {}", dest.display()));
    Ok(())
}

/// Point the `Exec` and `Icon` lines of a desktop entry at the bundled binary.
///
/// The scaffolded entry describes the project as it sits in the repository; in
/// the bundle the executable lives next to the entry, so both keys become the
/// package name.
fn retarget_desktop_entry(contents: &str, pkg_name: &str) -> String {
    let mut out = String::with_capacity(contents.len());
    for line in contents.lines() {
        if line.starts_with("Exec=") {
            out.push_str(&format!("Exec={pkg_name}"));
        } else if line.starts_with("Icon=") {
            out.push_str("Icon=app");
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// JDK feature releases the Android scaffold builds with, most preferred first.
///
/// Gradle 9 and the Android Gradle plugin both require JDK 17 as a minimum, so
/// older runtimes (8, 11) are deliberately never selected — picking one would
/// only trade a clear "no JDK found" into an opaque Gradle failure.
const SUPPORTED_JDK_VERSIONS: &[&str] = &["17", "21", "23"];

/// Locate a Gradle-compatible `JAVA_HOME` on macOS, preferring LTS releases.
pub(crate) fn resolve_compatible_java_home() -> Option<String> {
    if cfg!(target_os = "macos") {
        for version in SUPPORTED_JDK_VERSIONS {
            let Ok(output) = Command::new("/usr/libexec/java_home")
                .arg("-v")
                .arg(version)
                .output()
            else {
                continue;
            };
            if !output.status.success() {
                continue;
            }
            if let Ok(path) = String::from_utf8(output.stdout) {
                return Some(path.trim().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;

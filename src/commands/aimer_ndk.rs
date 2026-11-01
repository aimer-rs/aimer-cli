//! `aimer_ndk`: Aimer's own replacement for the `cargo-ndk` subcommand.
//!
//! Compiling Rust for Android needs cargo to be pointed at the NDK's clang
//! toolchain. `cargo-ndk` does that by exporting a handful of environment
//! variables and then running cargo; this module does the same in-process, so
//! Aimer no longer depends on a separately installed `cargo-ndk`:
//!
//! * [`Ndk::locate`] finds the NDK (`ANDROID_NDK_HOME` & co., then the SDK's
//!   `ndk/<version>` directories) and checks that it is recent enough.
//! * [`Ndk::envs`] derives the variables for one [`NdkTarget`]: the `cc` crate's
//!   `CC`/`CXX`/`AR`/`RANLIB`, cargo's per-target linker and archiver, and the
//!   ones `bindgen` and `clang-sys` read.
//! * [`Ndk::cargo_command`] returns a plain `cargo build --target <triple>`
//!   invocation carrying that environment.
//!
//! The NDK ships one clang wrapper per API level (`aarch64-linux-android24-clang`)
//! that already knows its `--target`, so unlike `cargo-ndk` no linker shim is
//! needed.

mod target;

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

pub(crate) use target::NdkTarget;

#[cfg(test)]
mod tests;

/// The Android API level binaries are linked against. It matches the `minSdk`
/// of the generated Android project.
pub(crate) const DEFAULT_PLATFORM: u8 = 24;

/// Oldest NDK major version with the clang toolchain layout this module uses.
const MIN_NDK_MAJOR: u32 = 23;

/// NDK releases up to this one do not link 64-bit libraries for 16 KiB pages
/// by default, which Android 15+ devices expect.
const LAST_NDK_WITHOUT_16K_PAGES: u32 = 27;

#[cfg(target_os = "macos")]
const HOST_TAG: &str = "darwin-x86_64";
#[cfg(target_os = "windows")]
const HOST_TAG: &str = "windows-x86_64";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const HOST_TAG: &str = "linux-x86_64";

/// No NDK could be found anywhere [`Ndk::locate`] looks.
#[derive(Debug, thiserror::Error)]
#[error(
    "Android NDK not found. Run `aimer components add ndk` to install it, \
     or set ANDROID_NDK_HOME to an existing NDK directory."
)]
pub(crate) struct NotFound;

/// Where `aimer components add ndk` keeps the NDKs it installs under the Aimer
/// directory `root`: one `<version>` directory each.
pub(crate) fn managed_dir(root: &Path) -> PathBuf {
    root.join("ndk")
}

/// Environment variables naming an NDK directory, most specific first.
const NDK_HOME_VARS: [&str; 4] = [
    "ANDROID_NDK_HOME",
    "ANDROID_NDK_ROOT",
    "NDK_HOME",
    "ANDROID_NDK",
];

/// Environment variables naming the Android SDK, most specific first.
const SDK_HOME_VARS: [&str; 2] = ["ANDROID_HOME", "ANDROID_SDK_ROOT"];

/// An installed Android NDK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ndk {
    home: PathBuf,
    version: String,
    major: u32,
}

impl Ndk {
    /// Find the NDK to build with.
    pub(crate) fn locate() -> anyhow::Result<Self> {
        for var in NDK_HOME_VARS {
            if let Some(home) = env::var_os(var).filter(|v| !v.is_empty()) {
                return Self::at(Path::new(&home))
                    .with_context(|| format!("{var} points at an unusable NDK"));
            }
        }
        for sdk in sdk_roots() {
            if let Some(home) = newest_ndk_in(&sdk.join("ndk")) {
                return Self::at(&home);
            }
            let bundle = sdk.join("ndk-bundle");
            if bundle.is_dir() {
                return Self::at(&bundle);
            }
        }
        if let Ok(root) = crate::commands::components::aimer_root()
            && let Some(home) = newest_ndk_in(&managed_dir(&root))
        {
            return Self::at(&home);
        }
        Err(NotFound.into())
    }

    /// Open the NDK installed at `home`, checking it is recent enough.
    pub(crate) fn at(home: &Path) -> anyhow::Result<Self> {
        let properties = home.join("source.properties");
        let text = std::fs::read_to_string(&properties).with_context(|| {
            format!(
                "{} is not an Android NDK (no source.properties)",
                home.display()
            )
        })?;
        let version = parse_revision(&text)
            .with_context(|| format!("no Pkg.Revision in {}", properties.display()))?;
        let major = version
            .split('.')
            .next()
            .and_then(|m| m.parse::<u32>().ok())
            .with_context(|| format!("unreadable NDK version '{version}'"))?;
        if major < MIN_NDK_MAJOR {
            bail!(
                "NDK r{major} is too old; Aimer needs r{MIN_NDK_MAJOR} or newer \
                 ({})",
                home.display()
            );
        }
        Ok(Self {
            home: home.to_path_buf(),
            version,
            major,
        })
    }

    /// The `Pkg.Revision` of the NDK, e.g. `27.0.12077973`.
    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    fn toolchain(&self) -> PathBuf {
        self.home
            .join("toolchains")
            .join("llvm")
            .join("prebuilt")
            .join(HOST_TAG)
    }

    fn tool(&self, name: &str) -> PathBuf {
        let name = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        };
        self.toolchain().join("bin").join(name)
    }

    /// The clang wrapper bound to `target` and `platform`, e.g.
    /// `aarch64-linux-android24-clang`.
    fn clang(&self, target: NdkTarget, platform: u8, driver: &str) -> PathBuf {
        let name = format!("{}{platform}-{driver}", target.clang_triple());
        let file = if cfg!(windows) {
            format!("{name}.cmd")
        } else {
            name
        };
        self.toolchain().join("bin").join(file)
    }

    /// The environment that makes cargo, the `cc` crate and `bindgen` use the
    /// NDK toolchain for `target`.
    pub(crate) fn envs(&self, target: NdkTarget, platform: u8) -> BTreeMap<String, OsString> {
        let triple = target.triple();
        let under = triple.replace('-', "_");
        let cargo_key = |key: &str| format!("CARGO_TARGET_{under}_{key}").to_uppercase();

        let clang = self.clang(target, platform, "clang");
        let clangxx = self.clang(target, platform, "clang++");
        let ar = self.tool("llvm-ar");
        let ranlib = self.tool("llvm-ranlib");
        let sysroot = self.toolchain().join("sysroot");
        let sysroot_triple = target.sysroot_triple();

        let mut envs: BTreeMap<String, OsString> = BTreeMap::new();
        let mut put = |key: String, value: OsString| {
            envs.insert(key, value);
        };
        put(format!("CC_{under}"), clang.clone().into());
        put(format!("CXX_{under}"), clangxx.into());
        put(format!("AR_{under}"), ar.clone().into());
        put(format!("RANLIB_{under}"), ranlib.into());
        put(cargo_key("AR"), ar.into());
        put(cargo_key("LINKER"), clang.clone().into());
        put("CLANG_PATH".to_string(), clang.into());
        put("ANDROID_PLATFORM".to_string(), platform.to_string().into());
        put("ANDROID_ABI".to_string(), target.abi().into());
        put("ANDROID_NDK_HOME".to_string(), self.home.clone().into());
        put("AIMER_NDK_SYSROOT".to_string(), sysroot.clone().into());
        put("AIMER_NDK_VERSION".to_string(), self.version.clone().into());
        put(
            format!("BINDGEN_EXTRA_CLANG_ARGS_{under}"),
            format!(
                "--target={}{platform} --sysroot={} -I{}",
                target.clang_triple(),
                sysroot.display(),
                sysroot
                    .join("usr")
                    .join("include")
                    .join(sysroot_triple)
                    .display()
            )
            .replace('\\', "/")
            .into(),
        );
        if let Some(libclang) = self.libclang_dir() {
            put("LIBCLANG_PATH".to_string(), libclang.into());
        }
        if let Some(rustflags) = self.page_size_rustflags(target, &cargo_key("RUSTFLAGS")) {
            put(cargo_key("RUSTFLAGS"), rustflags.into());
        }
        envs
    }

    /// 64-bit libraries must be linked for 16 KiB pages; NDK r28+ does it by
    /// default, older ones need the linker told.
    ///
    /// Skipped when `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` is set, as cargo
    /// ignores the per-target flags then. Flags already exported through
    /// `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` are kept.
    fn page_size_rustflags(&self, target: NdkTarget, key: &str) -> Option<String> {
        if !target.is_64bit() || self.major > LAST_NDK_WITHOUT_16K_PAGES {
            return None;
        }
        if env::var_os("RUSTFLAGS").is_some() || env::var_os("CARGO_ENCODED_RUSTFLAGS").is_some() {
            return None;
        }
        let flag = "-C link-arg=-Wl,-z,max-page-size=16384";
        Some(match env::var(key) {
            Ok(existing) if !existing.trim().is_empty() => format!("{existing} {flag}"),
            _ => flag.to_string(),
        })
    }

    /// The directory holding `libclang`, for `bindgen`.
    fn libclang_dir(&self) -> Option<PathBuf> {
        let prebuilt = self.toolchain();
        ["lib", "lib64", "bin", "musl/lib"]
            .iter()
            .map(|suffix| prebuilt.join(suffix))
            .find(|dir| contains_libclang(dir))
    }

    /// `cargo build --lib --target <triple> [--release]` with the NDK
    /// environment for `target` applied.
    pub(crate) fn cargo_command(&self, target: NdkTarget, release: bool) -> Command {
        let mut cmd = plain_cargo_command(target.triple(), release);
        cmd.envs(self.envs(target, DEFAULT_PLATFORM));
        cmd
    }
}

/// The cargo invocation for the `triple`, without any NDK environment.
fn plain_cargo_command(triple: &str, release: bool) -> Command {
    let mut cmd = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.args(["build", "--lib", "--target", triple]);
    if release {
        cmd.arg("--release");
    }
    cmd
}

/// The Android library build for the ABI or triple named `target`.
///
/// This is what replaces `cargo ndk -t <target> build --lib`. The command is
/// returned even when no NDK can be found (or `target` is not an Android one),
/// just without the NDK environment, so building it cannot fail; callers that
/// run it should surface the reason first through [`Ndk::locate`].
pub(crate) fn build_command(target: &str, release: bool) -> Command {
    match (NdkTarget::parse(target), Ndk::locate()) {
        (Some(target), Ok(ndk)) => ndk.cargo_command(target, release),
        (Some(target), Err(_)) => plain_cargo_command(target.triple(), release),
        (None, _) => plain_cargo_command(target, release),
    }
}

/// The value of `Pkg.Revision` in an NDK's `source.properties`.
fn parse_revision(properties: &str) -> Option<String> {
    properties.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "Pkg.Revision").then(|| value.trim().to_string())
    })
}

fn contains_libclang(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().starts_with("libclang."))
}

/// Android SDK directories worth searching for an NDK, best guess first.
fn sdk_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = SDK_HOME_VARS
        .iter()
        .filter_map(|var| env::var_os(var))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .collect();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Library").join("Android").join("sdk"));
        roots.push(home.join("Android").join("Sdk"));
    }
    if let Some(local) = dirs::data_local_dir() {
        roots.push(local.join("Android").join("Sdk"));
    }
    roots.dedup();
    roots
}

/// The highest-versioned NDK inside an SDK's `ndk` directory.
fn newest_ndk_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().join("source.properties").is_file())
        .max_by_key(|entry| version_key(&entry.file_name().to_string_lossy()))
        .map(|entry| entry.path())
}

/// A directory name like `27.0.12077973` as numbers, so `27.10` sorts after
/// `27.9`.
pub(crate) fn version_key(name: &str) -> Vec<u64> {
    name.split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

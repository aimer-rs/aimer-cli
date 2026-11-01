/// An Android ABI the NDK can compile for.
///
/// Accepts both spellings in use across the tooling: the Android ABI name
/// (`arm64-v8a`) and the Rust target triple (`aarch64-linux-android`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NdkTarget {
    ArmeabiV7a,
    Arm64V8a,
    X86,
    X86_64,
}

impl NdkTarget {
    /// Parse an Android ABI name or a Rust target triple.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "armeabi-v7a" | "armv7-linux-androideabi" => Self::ArmeabiV7a,
            "arm64-v8a" | "aarch64-linux-android" => Self::Arm64V8a,
            "x86" | "i686-linux-android" => Self::X86,
            "x86_64" | "x86_64-linux-android" => Self::X86_64,
            _ => return None,
        })
    }

    /// The Android ABI name, which is also the `jniLibs` directory name.
    pub(crate) fn abi(self) -> &'static str {
        match self {
            Self::ArmeabiV7a => "armeabi-v7a",
            Self::Arm64V8a => "arm64-v8a",
            Self::X86 => "x86",
            Self::X86_64 => "x86_64",
        }
    }

    /// The Rust target triple handed to `cargo --target`.
    pub(crate) fn triple(self) -> &'static str {
        match self {
            Self::ArmeabiV7a => "armv7-linux-androideabi",
            Self::Arm64V8a => "aarch64-linux-android",
            Self::X86 => "i686-linux-android",
            Self::X86_64 => "x86_64-linux-android",
        }
    }

    /// The triple the NDK's clang wrappers are named after. It only differs
    /// from the Rust one on 32-bit ARM, where the NDK says `armv7a`.
    pub(crate) fn clang_triple(self) -> &'static str {
        match self {
            Self::ArmeabiV7a => "armv7a-linux-androideabi",
            other => other.triple(),
        }
    }

    /// The triple naming the sysroot's per-architecture library directories.
    pub(crate) fn sysroot_triple(self) -> &'static str {
        match self {
            Self::ArmeabiV7a => "arm-linux-androideabi",
            other => other.triple(),
        }
    }

    /// Whether the ABI is a 64-bit one, and so subject to the page-size rules.
    pub(crate) fn is_64bit(self) -> bool {
        matches!(self, Self::Arm64V8a | Self::X86_64)
    }
}

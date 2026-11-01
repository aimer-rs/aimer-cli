use super::*;

/// A directory shaped like an NDK of the given `Pkg.Revision`.
fn fake_ndk(revision: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("source.properties"),
        format!("Pkg.Desc = Android NDK\nPkg.Revision = {revision}\n"),
    )
    .unwrap();
    dir
}

fn env_of(envs: &BTreeMap<String, OsString>, key: &str) -> String {
    envs.get(key)
        .unwrap_or_else(|| panic!("{key} missing from {envs:?}"))
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn targets_parse_from_abi_names_and_rust_triples() {
    for (name, abi, triple) in [
        ("arm64-v8a", "arm64-v8a", "aarch64-linux-android"),
        (
            "aarch64-linux-android",
            "arm64-v8a",
            "aarch64-linux-android",
        ),
        ("armeabi-v7a", "armeabi-v7a", "armv7-linux-androideabi"),
        ("x86", "x86", "i686-linux-android"),
        ("x86_64-linux-android", "x86_64", "x86_64-linux-android"),
    ] {
        let target = NdkTarget::parse(name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(target.abi(), abi);
        assert_eq!(target.triple(), triple);
    }
    assert_eq!(NdkTarget::parse("riscv64"), None);
}

#[test]
fn arm32_uses_the_armv7a_clang_and_the_arm_sysroot() {
    let target = NdkTarget::ArmeabiV7a;
    assert_eq!(target.clang_triple(), "armv7a-linux-androideabi");
    assert_eq!(target.sysroot_triple(), "arm-linux-androideabi");
    assert_eq!(NdkTarget::Arm64V8a.clang_triple(), "aarch64-linux-android");
    assert!(!target.is_64bit());
    assert!(NdkTarget::X86_64.is_64bit());
}

#[test]
fn the_revision_is_read_from_source_properties() {
    let ndk = Ndk::at(fake_ndk("27.0.12077973").path()).unwrap();
    assert_eq!(ndk.version(), "27.0.12077973");
}

#[test]
fn an_old_ndk_is_refused() {
    let err = Ndk::at(fake_ndk("21.4.7075529").path()).unwrap_err();
    assert!(err.to_string().contains("too old"), "{err}");
}

#[test]
fn a_directory_that_is_not_an_ndk_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let err = Ndk::at(dir.path()).unwrap_err();
    assert!(err.to_string().contains("not an Android NDK"), "{err}");
}

#[test]
fn the_newest_ndk_of_an_sdk_wins() {
    let sdk = tempfile::tempdir().unwrap();
    for version in ["26.1.10909125", "27.10.1", "27.9.1"] {
        let dir = sdk.path().join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("source.properties"), "Pkg.Revision = 27.0.0\n").unwrap();
    }
    // Not an NDK: no `source.properties`.
    std::fs::create_dir_all(sdk.path().join("99.0.0")).unwrap();

    let newest = newest_ndk_in(sdk.path()).unwrap();
    assert!(newest.ends_with("27.10.1"), "{newest:?}");
    assert_eq!(newest_ndk_in(&sdk.path().join("missing")), None);
}

#[test]
fn the_environment_points_cargo_and_cc_at_the_ndk_toolchain() {
    let home = fake_ndk("28.0.12433566");
    let ndk = Ndk::at(home.path()).unwrap();
    let envs = ndk.envs(NdkTarget::Arm64V8a, 24);
    let bin = format!(
        "{}/toolchains/llvm/prebuilt/{HOST_TAG}/bin",
        home.path().display()
    );
    let bin = bin.replace('\\', "/");

    let clang = env_of(&envs, "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER");
    assert!(clang.starts_with(&bin), "{clang}");
    assert!(clang.contains("aarch64-linux-android24-clang"), "{clang}");
    assert_eq!(env_of(&envs, "CC_aarch64_linux_android"), clang);
    assert_eq!(env_of(&envs, "CLANG_PATH"), clang);
    assert!(env_of(&envs, "CXX_aarch64_linux_android").contains("android24-clang++"));
    assert!(env_of(&envs, "AR_aarch64_linux_android").contains("llvm-ar"));
    assert!(env_of(&envs, "RANLIB_aarch64_linux_android").contains("llvm-ranlib"));
    assert!(env_of(&envs, "CARGO_TARGET_AARCH64_LINUX_ANDROID_AR").contains("llvm-ar"));
    assert_eq!(env_of(&envs, "ANDROID_PLATFORM"), "24");
    assert_eq!(env_of(&envs, "ANDROID_ABI"), "arm64-v8a");
    assert_eq!(env_of(&envs, "AIMER_NDK_VERSION"), "28.0.12433566");
}

#[test]
fn bindgen_is_given_the_sysroot_of_the_target() {
    let home = fake_ndk("28.0.12433566");
    let ndk = Ndk::at(home.path()).unwrap();
    let envs = ndk.envs(NdkTarget::ArmeabiV7a, 24);
    let args = env_of(&envs, "BINDGEN_EXTRA_CLANG_ARGS_armv7_linux_androideabi");
    assert!(
        args.contains("--target=armv7a-linux-androideabi24"),
        "{args}"
    );
    assert!(args.contains("--sysroot="), "{args}");
    assert!(
        args.contains("/usr/include/arm-linux-androideabi"),
        "{args}"
    );
    // The 32-bit build never needs the page-size flag.
    assert!(!envs.contains_key("CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_RUSTFLAGS"));
}

#[test]
fn recent_ndks_leave_the_page_size_to_the_linker() {
    let home = fake_ndk("28.0.12433566");
    let ndk = Ndk::at(home.path()).unwrap();
    let envs = ndk.envs(NdkTarget::Arm64V8a, 24);
    assert!(!envs.contains_key("CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS"));
}

#[test]
fn older_ndks_link_64_bit_libraries_for_16k_pages() {
    let home = fake_ndk("27.0.12077973");
    let ndk = Ndk::at(home.path()).unwrap();
    let envs = ndk.envs(NdkTarget::Arm64V8a, 24);
    // Cargo ignores the per-target flags when a global one is set, so the
    // module leaves those builds alone.
    if env::var_os("RUSTFLAGS").is_none() && env::var_os("CARGO_ENCODED_RUSTFLAGS").is_none() {
        let flags = env_of(&envs, "CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS");
        assert!(flags.contains("max-page-size=16384"), "{flags}");
    }
}

#[test]
fn libclang_is_found_next_to_the_toolchain() {
    let home = fake_ndk("28.0.12433566");
    let ndk = Ndk::at(home.path()).unwrap();
    assert_eq!(ndk.libclang_dir(), None);

    let lib = ndk.toolchain().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("libclang.dylib"), "").unwrap();
    assert_eq!(ndk.libclang_dir(), Some(lib));
}

#[test]
fn the_cargo_command_builds_the_library_for_the_triple() {
    let home = fake_ndk("28.0.12433566");
    let ndk = Ndk::at(home.path()).unwrap();
    let args = |release| -> Vec<String> {
        ndk.cargo_command(NdkTarget::X86_64, release)
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect()
    };
    assert_eq!(
        args(false),
        ["build", "--lib", "--target", "x86_64-linux-android"]
    );
    assert_eq!(
        args(true),
        [
            "build",
            "--lib",
            "--target",
            "x86_64-linux-android",
            "--release"
        ]
    );
}

#[test]
fn the_build_command_accepts_an_abi_or_a_triple() {
    let args = |target: &str| -> Vec<String> {
        build_command(target, false)
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect()
    };
    assert_eq!(args("arm64-v8a")[3], "aarch64-linux-android");
    assert_eq!(args("aarch64-linux-android")[3], "aarch64-linux-android");
}

#[test]
fn version_keys_compare_numerically() {
    assert!(version_key("27.10.1") > version_key("27.9.9"));
    assert!(version_key("28.0.0") > version_key("27.99.99"));
}

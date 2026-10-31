use std::collections::HashMap;

use super::*;
use crate::commands::package::test_support::{SpyReporter, args_of};

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |key| map.get(key).cloned()
}

#[test]
fn iscc_is_taken_from_the_environment_path_or_the_inno_setup_folder() {
    let set = env_of(&[(ENV_ISCC, "C:/tools/ISCC.exe")]);
    assert_eq!(
        locate_iscc(&set, &|p| p == Path::new("C:/tools/ISCC.exe")).unwrap(),
        Path::new("C:/tools/ISCC.exe")
    );

    let on_path = env_of(&[("PATH", "/a:/b")]);
    assert_eq!(
        locate_iscc(&on_path, &|p| p == Path::new("/b/ISCC.exe")).unwrap(),
        Path::new("/b/ISCC.exe")
    );

    let installed = env_of(&[("ProgramFiles(x86)", "/pf86"), ("PATH", "/a")]);
    assert_eq!(
        locate_iscc(&installed, &|p| p
            == Path::new("/pf86/Inno Setup 6/ISCC.exe"))
        .unwrap(),
        Path::new("/pf86/Inno Setup 6/ISCC.exe")
    );
    let per_user = env_of(&[("LOCALAPPDATA", "/local")]);
    assert_eq!(
        locate_iscc(&per_user, &|p| p
            == Path::new("/local/Programs/Inno Setup 6/ISCC.exe"))
        .unwrap(),
        Path::new("/local/Programs/Inno Setup 6/ISCC.exe")
    );
}

#[test]
fn a_missing_iscc_says_where_to_get_it() {
    let err = locate_iscc(&env_of(&[("PATH", "/a")]), &|_| false)
        .unwrap_err()
        .to_string();

    assert!(err.contains("jrsoftware.org"), "{err}");
    assert!(err.contains("6.3"), "{err}");
    assert!(err.contains(ENV_ISCC), "{err}");
}

#[test]
fn wix_is_taken_from_the_environment_path_or_the_dotnet_tools_folder() {
    let on_path = env_of(&[("PATH", "/a")]);
    assert_eq!(
        locate_wix(&on_path, &|p| p == Path::new("/a/wix.exe")).unwrap(),
        Path::new("/a/wix.exe")
    );

    let global = env_of(&[("USERPROFILE", "/home/me")]);
    assert_eq!(
        locate_wix(&global, &|p| p
            == Path::new("/home/me/.dotnet/tools/wix.exe"))
        .unwrap(),
        Path::new("/home/me/.dotnet/tools/wix.exe")
    );
}

#[test]
fn a_missing_wix_says_how_to_install_it_and_that_wix_3_is_not_supported() {
    let err = locate_wix(&env_of(&[]), &|_| false)
        .unwrap_err()
        .to_string();

    assert!(err.contains("dotnet tool install --global wix"), "{err}");
    assert!(err.contains("candle"), "{err}");
    assert!(err.contains(ENV_WIX), "{err}");
}

#[test]
fn architectures_use_the_names_windows_packages_use() {
    assert_eq!(arch_name("x86_64"), "x64");
    assert_eq!(arch_name("aarch64"), "arm64");
    assert_eq!(arch_name("x86"), "x86");
    assert_eq!(windows_arch(), arch_name(std::env::consts::ARCH));
}

#[test]
fn packages_are_named_after_app_version_and_architecture() {
    let name = |format, version| output_name(format, "My App", version, "x64");

    assert_eq!(
        name(PackageFormat::Exe, "1.2.3"),
        "My App-1.2.3-x64-setup.exe"
    );
    assert_eq!(name(PackageFormat::Msi, "1.2.3"), "My App-1.2.3-x64.msi");
    assert_eq!(
        name(PackageFormat::Zip, "1.2.3"),
        "My App-1.2.3-windows-x64.zip"
    );
    assert_eq!(name(PackageFormat::Exe, ""), "My App-x64-setup.exe");
}

#[test]
fn the_installer_id_is_a_valid_guid_that_depends_only_on_the_seed() {
    let id = stable_guid("aimer-inno:com.example.app");

    assert_eq!(id, stable_guid("aimer-inno:com.example.app"));
    assert_ne!(id, stable_guid("aimer-inno:com.example.other"));
    assert_ne!(id, stable_guid("aimer-wix:com.example.app"));
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(
        parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
        [8, 4, 4, 4, 12]
    );
    assert!(
        id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()),
        "{id}"
    );
    assert!(!id.chars().any(|c| c.is_ascii_lowercase()), "{id}");
    assert!(parts[2].starts_with('5'), "version 5: {id}");
    assert!(
        ["8", "9", "A", "B"].contains(&&parts[3][..1]),
        "RFC 4122 variant: {id}"
    );
}

fn bundle(root: &Path) -> PathBuf {
    let bundle = root.join("bundle");
    std::fs::create_dir_all(bundle.join("assets")).unwrap();
    std::fs::write(bundle.join("my_app.exe"), b"MZ").unwrap();
    std::fs::write(bundle.join("assets/data.json"), b"{}").unwrap();
    std::fs::write(bundle.join("app.ico"), b"ico").unwrap();
    bundle
}

fn job<'a>(
    format: PackageFormat,
    bundle: &'a Path,
    dist: &'a str,
    tool: Option<&'a Path>,
) -> WindowsJob<'a> {
    WindowsJob {
        format,
        pkg_name: "my_app",
        app_name: "My App",
        version: "1.2.3",
        publisher: "Jane Doe",
        group: "com.example.myapp",
        bundle,
        dist,
        tool,
    }
}

/// A spy that writes the package a real tool would leave: ISCC puts it in the
/// `OutputDir` of its script, `wix` at its `-o`.
fn writing_spy() -> SpyReporter {
    SpyReporter::with_hook(|cmd| {
        let args = args_of(cmd);
        if let Some(at) = args.iter().position(|a| a == "-o") {
            std::fs::write(&args[at + 1], b"msi").unwrap();
        } else {
            let script = std::fs::read_to_string(args.last().unwrap()).unwrap();
            let value = |key: &str| {
                script
                    .lines()
                    .find_map(|l| l.strip_prefix(key))
                    .unwrap()
                    .to_string()
            };
            let path = Path::new(&value("OutputDir="))
                .join(format!("{}.exe", value("OutputBaseFilename=")));
            std::fs::write(path, b"exe").unwrap();
        }
    })
}

#[test]
fn a_zip_is_the_bundle_under_a_clean_folder() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let spy = SpyReporter::new();

    let out = package_bundle(&job(PackageFormat::Zip, &bundle, dist, None), &spy).unwrap();

    assert!(
        out.ends_with(&format!("My App-1.2.3-windows-{}.zip", windows_arch())),
        "{out}"
    );
    let zip = std::fs::read(&out).unwrap();
    let text = String::from_utf8_lossy(&zip);
    assert!(text.contains("my_app-1.2.3/my_app.exe"), "{text:.0}");
    assert!(text.contains("my_app-1.2.3/assets/data.json"));
    assert!(spy.steps().is_empty(), "a zip needs no external tool");
    assert!(!Path::new(dist).join(WORK_DIR_NAME).exists());
}

#[test]
fn a_setup_exe_is_scripted_and_handed_to_iscc() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let tool = Path::new("C:/tools/ISCC.exe");
    let script_seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let probe = script_seen.clone();
    let spy = SpyReporter::with_hook(move |cmd| {
        let script = std::fs::read_to_string(args_of(cmd).last().unwrap()).unwrap();
        let out = format!(
            "{}/{}.exe",
            script
                .lines()
                .find_map(|l| l.strip_prefix("OutputDir="))
                .unwrap(),
            script
                .lines()
                .find_map(|l| l.strip_prefix("OutputBaseFilename="))
                .unwrap()
        );
        std::fs::write(out, b"exe").unwrap();
        *probe.lock().unwrap() = script;
    });

    let out = package_bundle(&job(PackageFormat::Exe, &bundle, dist, Some(tool)), &spy).unwrap();

    assert!(
        out.ends_with(&format!("My App-1.2.3-{}-setup.exe", windows_arch())),
        "{out}"
    );
    assert_eq!(spy.programs(), ["C:/tools/ISCC.exe"]);
    let script = script_seen.lock().unwrap().clone();
    assert!(script.contains("AppName=My App"), "{script}");
    assert!(script.contains("AppPublisher=Jane Doe"), "{script}");
    assert!(
        script.contains(&format!(
            "AppId={{{{{}}}",
            stable_guid("aimer-inno:com.example.myapp")
        )),
        "{script}"
    );
    assert!(!Path::new(dist).join(WORK_DIR_NAME).exists());
}

#[test]
fn an_msi_is_generated_and_built_by_wix() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let tool = Path::new("C:/dotnet/wix.exe");
    let spy = writing_spy();

    let out = package_bundle(&job(PackageFormat::Msi, &bundle, dist, Some(tool)), &spy).unwrap();

    assert!(
        out.ends_with(&format!("My App-1.2.3-{}.msi", windows_arch())),
        "{out}"
    );
    let step = &spy.steps()[0];
    assert_eq!(step.program, "C:/dotnet/wix.exe");
    assert_eq!(step.args[0], "build");
    assert!(step.args.last().unwrap().ends_with("package.wxs"));
    assert!(!Path::new(dist).join(WORK_DIR_NAME).exists());
}

#[test]
fn an_installer_without_its_tool_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let dist = dir.path().join("dist");
    for (format, word) in [(PackageFormat::Exe, "ISCC"), (PackageFormat::Msi, "wix")] {
        let err = package_bundle(
            &job(format, &bundle, dist.to_str().unwrap(), None),
            &SpyReporter::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains(word), "{err}");
    }
}

#[test]
fn a_msi_version_that_does_not_fit_stops_before_wix_runs() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let dist = dir.path().join("dist");
    let mut bad = job(
        PackageFormat::Msi,
        &bundle,
        dist.to_str().unwrap(),
        Some(Path::new("wix")),
    );
    bad.version = "1.2.99999";
    let spy = SpyReporter::new();

    let err = package_bundle(&bad, &spy).unwrap_err();

    assert!(err.to_string().contains("MSI version"), "{err}");
    assert!(spy.steps().is_empty());
}

#[test]
fn a_tool_that_produces_nothing_is_reported_and_leaves_no_scratch_folder() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let dist = dir.path().join("dist");

    let err = package_bundle(
        &job(
            PackageFormat::Msi,
            &bundle,
            dist.to_str().unwrap(),
            Some(Path::new("wix")),
        ),
        &SpyReporter::new(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("was not produced"), "{err}");
    assert!(!dist.join(WORK_DIR_NAME).exists());
}

#[test]
fn a_failing_tool_stops_the_package_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let dist = dir.path().join("dist");

    let err = package_bundle(
        &job(
            PackageFormat::Exe,
            &bundle,
            dist.to_str().unwrap(),
            Some(Path::new("ISCC.exe")),
        ),
        &SpyReporter::failing("ISCC"),
    )
    .unwrap_err();

    assert!(err.to_string().contains("ISCC failed"), "{err}");
    assert!(!dist.join(WORK_DIR_NAME).exists());
}

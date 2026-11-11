use std::collections::HashMap;

use super::*;
use crate::commands::package::test_support::{SpyReporter, args_of};
use crate::config::AndroidSigningMeta;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |key| map.get(key).cloned()
}

fn meta() -> AndroidMeta {
    AndroidMeta {
        signing: Some(AndroidSigningMeta {
            keystore: Some("keys/upload.jks".into()),
            key_alias: Some("upload".into()),
        }),
        version_code: None,
        ..AndroidMeta::default()
    }
}

const PASSWORD: (&str, &str) = (ENV_KEYSTORE_PASSWORD, "storepass");

fn resolve(
    meta: Option<&AndroidMeta>,
    env: &[(&str, &str)],
    version: &str,
) -> anyhow::Result<AndroidSigningPlan> {
    AndroidSigningPlan::resolve(meta, &env_of(env), version, Path::new("/proj"))
}

fn plan() -> AndroidSigningPlan {
    resolve(Some(&meta()), &[PASSWORD], "1.2.3").unwrap()
}

/// The keystore of `meta()` under the `/proj` root, joined the way the OS
/// joins paths (a backslash is inserted on Windows).
fn project_keystore() -> String {
    Path::new("/proj")
        .join("keys/upload.jks")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_manifest_settings_are_resolved_against_the_project_root() {
    let plan = plan();

    assert_eq!(plan.keystore, project_keystore());
    assert_eq!(plan.key_alias, "upload");
    assert_eq!(plan.keystore_password, "storepass");
    assert_eq!(plan.version_name.as_deref(), Some("1.2.3"));
}

#[test]
fn an_absolute_keystore_path_is_kept() {
    let env = [PASSWORD, (ENV_KEYSTORE, "/keys/other.jks")];
    let plan = resolve(Some(&meta()), &env, "1.0.0").unwrap();

    assert_eq!(plan.keystore, "/keys/other.jks");
}

#[test]
fn the_environment_overrides_the_manifest() {
    let env = [
        PASSWORD,
        (ENV_KEY_ALIAS, "release"),
        (ENV_VERSION_CODE, "77"),
    ];
    let plan = resolve(Some(&meta()), &env, "1.2.3").unwrap();

    assert_eq!(plan.key_alias, "release");
    assert_eq!(plan.version_code, 77);
}

#[test]
fn the_key_password_defaults_to_the_keystore_password() {
    assert_eq!(plan().key_password, "storepass");

    let env = [PASSWORD, (ENV_KEY_PASSWORD, "keypass")];
    let plan = resolve(Some(&meta()), &env, "1.2.3").unwrap();
    assert_eq!(plan.key_password, "keypass");
}

#[test]
fn every_missing_setting_names_where_to_set_it() {
    let no_keystore = resolve(None, &[PASSWORD], "1.0.0").unwrap_err().to_string();
    assert!(no_keystore.contains("[android.signing]"), "{no_keystore}");
    assert!(no_keystore.contains(ENV_KEYSTORE), "{no_keystore}");

    let only_keystore = AndroidMeta {
        signing: Some(AndroidSigningMeta {
            keystore: Some("k.jks".into()),
            key_alias: None,
        }),
        version_code: None,
        ..AndroidMeta::default()
    };
    let no_alias = resolve(Some(&only_keystore), &[PASSWORD], "1.0.0")
        .unwrap_err()
        .to_string();
    assert!(no_alias.contains(ENV_KEY_ALIAS), "{no_alias}");

    let no_password = resolve(Some(&meta()), &[], "1.0.0")
        .unwrap_err()
        .to_string();
    assert!(no_password.contains(ENV_KEYSTORE_PASSWORD), "{no_password}");
}

#[test]
fn a_password_in_the_manifest_is_not_a_thing() {
    // There is no manifest field for it; only the environment provides one.
    let err = resolve(Some(&meta()), &[], "1.0.0")
        .unwrap_err()
        .to_string();

    assert!(err.contains("environment variable"), "{err}");
}

#[test]
fn the_debug_output_never_shows_a_password() {
    let env = [PASSWORD, (ENV_KEY_PASSWORD, "keypass")];
    let plan = resolve(Some(&meta()), &env, "1.2.3").unwrap();

    let shown = format!("{plan:?}");
    assert!(
        !shown.contains("storepass") && !shown.contains("keypass"),
        "{shown}"
    );
    assert!(shown.contains("<redacted>"), "{shown}");
}

#[test]
fn the_version_code_comes_from_the_version_unless_given() {
    assert_eq!(version_code(None, "1.2.3").unwrap(), 1_002_003);
    assert_eq!(version_code(None, "0.1.0").unwrap(), 1_000);
    assert_eq!(version_code(None, "2").unwrap(), 2_000_000);
    assert_eq!(version_code(None, "1.2").unwrap(), 1_002_000);
    assert_eq!(version_code(None, "1.2.3-beta+4").unwrap(), 1_002_003);
    assert_eq!(version_code(Some(9), "1.2.3").unwrap(), 9);
    assert_eq!(version_code(Some(9), "").unwrap(), 9);
}

#[test]
fn a_version_that_cannot_give_a_code_says_how_to_set_one() {
    for version in ["", "1.2.3.4", "1.x.0", "1.1000.0", "v1.0.0"] {
        let err = version_code(None, version).unwrap_err().to_string();
        assert!(err.contains("version_code"), "{version:?}: {err}");
    }
}

#[test]
fn the_manifest_version_code_is_used_and_the_environment_wins() {
    let with_code = AndroidMeta {
        version_code: Some(500),
        ..meta()
    };
    assert_eq!(
        resolve(Some(&with_code), &[PASSWORD], "1.0.0")
            .unwrap()
            .version_code,
        500
    );
    let env = [PASSWORD, (ENV_VERSION_CODE, "600")];
    assert_eq!(
        resolve(Some(&with_code), &env, "1.0.0")
            .unwrap()
            .version_code,
        600
    );
    let bad = [PASSWORD, (ENV_VERSION_CODE, "abc")];
    assert!(resolve(Some(&with_code), &bad, "1.0.0").is_err());
}

#[test]
fn a_missing_keystore_fails_the_preflight() {
    let err = plan().preflight(None).unwrap_err().to_string();

    assert!(err.contains(&project_keystore()), "{err}");
}

#[test]
fn keytool_checks_the_alias_with_the_password_from_the_environment() {
    let cmd = plan().keytool_command(Some("/jdk"));

    assert_eq!(
        cmd.get_program(),
        Path::new("/jdk").join("bin/keytool").as_os_str()
    );
    assert_eq!(
        args_of(&cmd),
        [
            "-list",
            "-keystore",
            project_keystore().as_str(),
            "-alias",
            "upload",
            "-storepass:env",
            ENV_KEYSTORE_PASSWORD,
        ]
    );
    assert!(
        !args_of(&cmd)
            .iter()
            .any(|a| a.contains("storepass") && a.contains('='))
    );
    assert_eq!(plan().keytool_command(None).get_program(), "keytool");
}

#[test]
fn gradle_gets_the_init_script_and_the_task_of_the_format() {
    let dir = tempfile::tempdir().unwrap();
    let job = ArtifactJob {
        format: PackageFormat::Aab,
        app_name: "My App",
        version: "1.2.3",
        project_dir: dir.path(),
        dist: "dist",
        java_home: Some("/jdk"),
    };

    let aab = gradle_command(&job, &plan(), Path::new("/w/init.gradle"));
    assert_eq!(args_of(&aab), ["-I", "/w/init.gradle", "bundleRelease"]);
    assert_eq!(aab.get_current_dir(), Some(dir.path()));
    assert!(
        aab.get_program()
            .to_string_lossy()
            .starts_with(dir.path().to_str().unwrap())
    );

    let apk_job = ArtifactJob {
        format: PackageFormat::Apk,
        ..job
    };
    let apk = gradle_command(&apk_job, &plan(), Path::new("/w/init.gradle"));
    assert_eq!(args_of(&apk), ["-I", "/w/init.gradle", "assembleRelease"]);
}

#[test]
fn the_signing_environment_reaches_gradle_but_not_the_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let job = ArtifactJob {
        format: PackageFormat::Aab,
        app_name: "App",
        version: "1.2.3",
        project_dir: dir.path(),
        dist: "dist",
        java_home: Some("/jdk"),
    };
    let cmd = gradle_command(&job, &plan(), Path::new("/w/init.gradle"));

    let env: HashMap<String, String> = cmd
        .get_envs()
        .filter_map(|(k, v)| Some((k.to_string_lossy().into(), v?.to_string_lossy().into())))
        .collect();
    assert_eq!(env[ENV_KEYSTORE], project_keystore());
    assert_eq!(env[ENV_KEY_ALIAS], "upload");
    assert_eq!(env[ENV_KEYSTORE_PASSWORD], "storepass");
    assert_eq!(env[ENV_KEY_PASSWORD], "storepass");
    assert_eq!(env[ENV_VERSION_CODE], "1002003");
    assert_eq!(env[ENV_VERSION_NAME], "1.2.3");
    assert_eq!(env["JAVA_HOME"], "/jdk");
    assert!(!args_of(&cmd).iter().any(|a| a.contains("storepass")));
}

#[test]
fn the_packages_are_found_where_gradle_leaves_them() {
    let root = Path::new("/p");

    assert_eq!(
        gradle_output(root, PackageFormat::Aab),
        Path::new("/p/app/build/outputs/bundle/release/app-release.aab")
    );
    assert_eq!(
        gradle_output(root, PackageFormat::Apk),
        Path::new("/p/app/build/outputs/apk/release/app-release.apk")
    );
}

#[test]
fn the_init_script_overrides_signing_and_version_in_finalize_dsl() {
    assert!(INIT_SCRIPT.contains("finalizeDsl"));
    assert!(INIT_SCRIPT.contains("AIMER_ANDROID_KEYSTORE_PASSWORD"));
    assert!(INIT_SCRIPT.contains("versionCode"));
}

/// A spy that writes the package a real Gradle run would leave behind.
fn building_spy() -> SpyReporter {
    SpyReporter::with_hook(|cmd| {
        let dir = cmd.get_current_dir().unwrap();
        let task = args_of(cmd).pop().unwrap();
        let format = if task == "bundleRelease" {
            PackageFormat::Aab
        } else {
            PackageFormat::Apk
        };
        let out = gradle_output(dir, format);
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(out, b"package").unwrap();
    })
}

struct Fixture {
    project: tempfile::TempDir,
    dist: tempfile::TempDir,
}

fn fixture() -> Fixture {
    Fixture {
        project: tempfile::tempdir().unwrap(),
        dist: tempfile::tempdir().unwrap(),
    }
}

fn package(fx: &Fixture, format: PackageFormat, spy: &SpyReporter) -> anyhow::Result<String> {
    let job = ArtifactJob {
        format,
        app_name: "My App",
        version: "1.2.3",
        project_dir: fx.project.path(),
        dist: fx.dist.path().to_str().unwrap(),
        java_home: None,
    };
    package_artifact(&job, &plan(), spy)
}

#[test]
fn an_aab_is_built_copied_and_named_after_the_version() {
    let fx = fixture();
    let spy = building_spy();

    let output = package(&fx, PackageFormat::Aab, &spy).unwrap();

    assert_eq!(
        output,
        format!("{}/My App-1.2.3.aab", fx.dist.path().display())
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"package");
    assert_eq!(spy.steps().len(), 1);
    assert_eq!(spy.steps()[0].action, "Gradle package for Android");
    assert!(!fx.dist.path().join(WORK_DIR_NAME).exists());
}

#[test]
fn an_apk_is_built_with_assemble_release() {
    let fx = fixture();
    let spy = building_spy();

    let output = package(&fx, PackageFormat::Apk, &spy).unwrap();

    assert!(output.ends_with("My App-1.2.3.apk"), "{output}");
    assert_eq!(spy.steps()[0].args.last().unwrap(), "assembleRelease");
}

#[test]
fn the_init_script_exists_while_gradle_runs() {
    let fx = fixture();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(false));
    let probe = seen.clone();
    let spy = SpyReporter::with_hook(move |cmd| {
        let args = args_of(cmd);
        let script = Path::new(&args[1]);
        *probe.lock().unwrap() = std::fs::read_to_string(script)
            .map(|text| text.contains("finalizeDsl"))
            .unwrap_or(false);
        let out = gradle_output(cmd.get_current_dir().unwrap(), PackageFormat::Aab);
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(out, b"package").unwrap();
    });

    package(&fx, PackageFormat::Aab, &spy).unwrap();

    assert!(*seen.lock().unwrap());
}

#[test]
fn a_package_left_by_an_earlier_run_is_never_taken_for_this_one() {
    let fx = fixture();
    let stale = gradle_output(fx.project.path(), PackageFormat::Aab);
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, b"stale").unwrap();

    // Gradle "succeeds" without producing anything.
    let err = package(&fx, PackageFormat::Aab, &SpyReporter::new()).unwrap_err();

    assert!(err.to_string().contains("was not produced"), "{err}");
    assert!(!stale.exists());
}

#[test]
fn a_failed_gradle_run_cleans_up_and_keeps_nothing() {
    let fx = fixture();
    let spy = SpyReporter::failing("Gradle package for Android");

    let err = package(&fx, PackageFormat::Aab, &spy).unwrap_err();

    assert!(err.to_string().contains("Gradle"), "{err}");
    assert!(!fx.dist.path().join(WORK_DIR_NAME).exists());
    assert!(!fx.dist.path().join("My App-1.2.3.aab").exists());
}

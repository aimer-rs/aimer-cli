use super::*;

fn answers(targets: Vec<&'static str>) -> Answers {
    Answers {
        app_name: "Demo App".to_string(),
        version: "0.3.0".to_string(),
        description: "a demo".to_string(),
        author: "tester".to_string(),
        group: "com.example.demo".to_string(),
        targets,
    }
}

fn cargo_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.3.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(dir.path().join("README.md"), "mine\n").unwrap();
    fs::write(dir.path().join(".gitignore"), "/target").unwrap();
    dir
}

fn init(dir: &Path, answers: &Answers) -> anyhow::Result<()> {
    let (document, package) = cargo_manifest::read(dir)?;
    initialize(dir, &package.name, document, answers)
}

#[test]
fn initializes_a_cargo_project_in_place() {
    let project = cargo_project();
    let dir = project.path();

    init(dir, &answers(vec!["web", "linux"])).unwrap();

    // Same scaffolds and manifest as `aimer create`.
    assert!(dir.join("builds/web/index.html").is_file());
    assert!(dir.join("builds/linux/demo.desktop").is_file());
    let manifest = AimerManifest::load_from(dir).unwrap().unwrap();
    assert_eq!(manifest.package.name, "Demo App");
    assert_eq!(manifest.package.version, "0.3.0");
    assert_eq!(manifest.package.group, "com.example.demo");
    assert_eq!(crate::config::resolve_cargo_package_name(dir), "demo");
    assert!(
        !dir.join("build.rs").exists(),
        "init must not add a build.rs"
    );

    // The package itself is adapted, not replaced.
    let cargo = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("name = \"demo\""), "{cargo}");
    assert!(cargo.contains("cdylib"), "{cargo}");
    assert!(cargo.contains("aimer"), "{cargo}");
    assert_eq!(
        fs::read_to_string(dir.join("src/main.rs")).unwrap(),
        "fn main() {}\n"
    );
    assert_eq!(fs::read_to_string(dir.join("README.md")).unwrap(), "mine\n");

    let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(gitignore.starts_with("/target\n"), "{gitignore}");
    assert!(gitignore.contains("builds/web/pkg"), "{gitignore}");
}

#[test]
fn refuses_an_already_initialized_project() {
    let project = cargo_project();
    let dir = project.path();
    init(dir, &answers(vec![])).unwrap();

    let err = init(dir, &answers(vec![])).unwrap_err();
    assert!(err.to_string().contains("aimer migrate"), "{err}");
}

#[test]
fn refuses_to_overwrite_an_existing_platform_folder() {
    let project = cargo_project();
    let dir = project.path();
    fs::create_dir_all(dir.join("builds/web")).unwrap();
    fs::write(dir.join("builds/web/index.html"), "mine").unwrap();
    let cargo_before = fs::read_to_string(dir.join("Cargo.toml")).unwrap();

    let err = init(dir, &answers(vec!["web"])).unwrap_err();

    assert!(
        err.to_string().contains("builds/web already exists"),
        "{err}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("builds/web/index.html")).unwrap(),
        "mine"
    );
    assert_eq!(
        fs::read_to_string(dir.join("Cargo.toml")).unwrap(),
        cargo_before
    );
    assert!(!dir.join("Aimer.toml").exists());
}

#[test]
fn undo_takes_back_what_was_written() {
    let project = cargo_project();
    let dir = project.path();
    let cargo_before = fs::read_to_string(dir.join("Cargo.toml")).unwrap();

    let mut undo = Undo::default();
    undo.created.push(dir.join("builds/linux"));
    scaffold_platforms(dir, "demo", "demo", "com.example", &["linux"]);
    fs::write(dir.join("Cargo.toml"), "changed").unwrap();
    undo.restore
        .push((dir.join("Cargo.toml"), cargo_before.clone()));
    undo.run();

    assert!(!dir.join("builds/linux").exists());
    assert_eq!(
        fs::read_to_string(dir.join("Cargo.toml")).unwrap(),
        cargo_before
    );
}

#[test]
fn gitignore_merge_adds_only_missing_build_rules() {
    let template = "# comment\nbuilds/web/pkg\nbuilds/ios/build\ntarget\nCargo.lock\n";

    let merged = merge_gitignore("/target\nbuilds/web/pkg", template).unwrap();
    assert_eq!(
        merged,
        "/target\nbuilds/web/pkg\n\n# aimer platform builds\nbuilds/ios/build\n"
    );

    assert_eq!(merge_gitignore(&merged, template), None);
    assert_eq!(
        merge_gitignore("", template).unwrap(),
        "# aimer platform builds\nbuilds/web/pkg\nbuilds/ios/build\n"
    );
}

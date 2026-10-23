pub mod android;
pub mod ios;
pub mod linux;
pub mod macos;
pub mod web;
pub mod window;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use inquire::ui::{Color, RenderConfig, Styled};
use inquire::{Confirm, MultiSelect, Text};

use crate::config::AimerManifest;
use crate::errors::AimerError;

macro_rules! prompt_abortable {
    ($prompt:expr) => {
        loop {
            match $prompt.prompt() {
                Ok(val) => break val,
                Err(inquire::error::InquireError::OperationInterrupted) => {
                    println!("press 'ctrl + c' again to exit");
                    crossterm::terminal::enable_raw_mode().unwrap();
                    if let Ok(crossterm::event::Event::Key(event)) = crossterm::event::read() {
                        if event.code == crossterm::event::KeyCode::Char('c')
                            && event
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            crossterm::terminal::disable_raw_mode().unwrap();
                            std::process::exit(1);
                        }
                    }
                    crossterm::terminal::disable_raw_mode().unwrap();
                }
                Err(inquire::error::InquireError::OperationCanceled) => {
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    };
}

pub fn execute(project_name: &str) -> anyhow::Result<()> {
    // Reject invalid names up-front, before any prompts or filesystem writes.
    validate_project_name(project_name)?;

    let config = RenderConfig {
        prompt_prefix: Styled::new("◆").with_fg(Color::LightCyan),
        highlighted_option_prefix: Styled::new("│  ❯").with_fg(Color::LightCyan),
        unhighlighted_option_prefix: Styled::new("│   ").with_fg(Color::DarkGrey),
        selected_checkbox: Styled::new("●").with_fg(Color::LightGreen),
        unselected_checkbox: Styled::new("○").with_fg(Color::DarkGrey),
        ..RenderConfig::default()
    };
    inquire::set_global_render_config(config);

    let current_dir = std::env::current_dir().context("failed to read the current directory")?;
    tracing::debug!("current_dir : {}", current_dir.display());
    println!("Creating project '{}'", project_name);

    let app_name = prompt_abortable!(
        Text::new("App name:")
            .with_default(project_name)
            .with_validator(validate_app_name_input)
    );
    let app_name = app_name.trim().to_string();
    let description = prompt_abortable!(Text::new("Description:"));
    let version = prompt_abortable!(Text::new("Version:").with_default("0.1.0"));
    let author = prompt_abortable!(Text::new("Author:"));
    let group = prompt_abortable!(
        Text::new("Group (e.g. com.example.app):").with_default("com.example.app")
    );
    let targets = prompt_abortable!(MultiSelect::new(
        "Targets:",
        vec!["macos", "windows", "linux", "android", "ios", "web"]
    ));

    println!(
        "\nProject config:\n- Name: {}\n- App name: {}\n- Version: {}\n- Description: {}\n- Author: {}\n- Group: {}\n- Targets: {:?}",
        project_name, app_name, version, description, author, group, targets
    );

    if !prompt_abortable!(Confirm::new("Is this okay?").with_default(true)) {
        println!("Aborted.");
        return Ok(());
    }

    let dir = PathBuf::from(project_name);
    if dir.exists() {
        println!("Directory '{}' already exists!", project_name);
        return Ok(());
    }

    // Run the scaffold inside a helper so that any failure cleans up the
    // partially created directory instead of leaving it behind.
    if let Err(err) = scaffold(
        &dir,
        project_name,
        &app_name,
        &version,
        &description,
        &author,
        &group,
        &targets,
    ) {
        let _ = fs::remove_dir_all(&dir);
        return Err(err).with_context(|| format!("failed to create project '{project_name}'"));
    }

    println!("Project '{}' created successfully.", project_name);
    Ok(())
}

/// Write the full project tree into `dir`. Returns an error (with context) on
/// the first failing filesystem operation so the caller can clean up.
fn scaffold(
    dir: &Path,
    project_name: &str,
    app_name: &str,
    version: &str,
    description: &str,
    author: &str,
    group: &str,
    targets: &[&str],
) -> anyhow::Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating directory {}", dir.display()))?;
    fs::create_dir_all(dir.join("src")).context("creating src directory")?;
    fs::create_dir_all(dir.join("builds")).context("creating builds directory")?;
    fs::create_dir_all(dir.join("builds/web")).context("creating builds/web directory")?;
    // fs::create_dir_all(dir.join("builds/build_src/src")).context("creating
    // builds/build_src/src directory")?;

    if targets.contains(&"android") {
        android::create(dir, project_name, app_name, group);
    }
    if targets.contains(&"ios") {
        ios::create(dir, project_name, app_name, group);
    }
    if targets.contains(&"macos") {
        macos::create(dir, project_name, app_name, group);
    }
    if targets.contains(&"windows") {
        window::create(dir, project_name, app_name, group);
    }
    if targets.contains(&"linux") {
        linux::create(dir, project_name, app_name, group);
    }
    if targets.contains(&"web") {
        web::create(dir, project_name, app_name, group);
    }

    fs::write(
        dir.join(".gitignore"),
        include_str!("../../templates/.gitignore.template"),
    )
    .context("writing .gitignore")?;

    // Cargo.toml
    fs::write(
        dir.join("Cargo.toml"),
        include_str!("../../templates/Cargo.toml.template")
            .replace("${project_name}", project_name)
            .replace("${version}", version),
    )
    .context("writing Cargo.toml")?;

    // src/lib.rs
    fs::write(
        dir.join("src/main.rs"),
        include_str!("../../templates/main.rs.template"),
    )
    .context("writing src/main.rs")?;

    // build.rs — iOS cdylib link workaround for the Swift-provided frame-driver
    // symbols (see the template for details).
    fs::write(
        dir.join("build.rs"),
        include_str!("../../templates/build.rs.template"),
    )
    .context("writing build.rs")?;

    fs::write(
        dir.join("README.md"),
        format!("# {}\n\n{}", app_name, description),
    )
    .context("writing README.md")?;

    // Persist project metadata so run/build can read it back instead of
    // re-parsing Cargo.toml. `package.name` is the application name the
    // bundles are shipped under; the Cargo package keeps the project name.
    AimerManifest::new(app_name, version, description, author, group)
        .write_to(dir)
        .context("writing Aimer.toml")?;

    Ok(())
}

/// Escape the characters that are significant in XML and HTML text so an
/// application name can be embedded in a generated manifest or page.
pub(crate) fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Reject an application name the platform bundles cannot be named after.
///
/// Unlike the project name this is a display name, so spaces are fine; it still
/// becomes a `.app` folder and an executable file name, so it must be non-empty
/// and free of path separators and the characters filesystems reserve.
fn validate_app_name(name: &str) -> Result<(), &'static str> {
    const RESERVED: &[char] = &[':', '*', '?', '"', '<', '>', '|'];
    let name = name.trim();
    if name.is_empty() {
        return Err("app name must not be empty");
    }
    if name == "." || name == ".." {
        return Err("app name must not be '.' or '..'");
    }
    if name.contains(['/', '\\']) {
        return Err("app name must not contain path separators");
    }
    if name.contains(RESERVED) {
        return Err("app name must not contain reserved characters (: * ? \" < > |)");
    }
    if name.contains(|c: char| c.is_control()) {
        return Err("app name must not contain control characters");
    }
    Ok(())
}

/// [`validate_app_name`] in the shape the `inquire` prompt expects.
fn validate_app_name_input(
    input: &str,
) -> Result<inquire::validator::Validation, inquire::CustomUserError> {
    Ok(match validate_app_name(input) {
        Ok(()) => inquire::validator::Validation::Valid,
        Err(message) => inquire::validator::Validation::Invalid(message.into()),
    })
}

/// Validate a project name before any prompts or filesystem writes.
///
/// Rejects empty names, path separators, whitespace, the `.`/`..` specials and
/// characters that are reserved on common filesystems.
pub fn validate_project_name(name: &str) -> Result<(), AimerError> {
    const RESERVED: &[char] = &[':', '*', '?', '"', '<', '>', '|'];
    let reject = |reason: &str| {
        Err(AimerError::InvalidProjectName(
            name.to_string(),
            reason.to_string(),
        ))
    };

    if name.trim().is_empty() {
        return reject("name must not be empty");
    }
    if name == "." || name == ".." {
        return reject("name must not be '.' or '..'");
    }
    if name.contains(['/', '\\']) {
        return reject("name must not contain path separators");
    }
    if name.chars().any(char::is_whitespace) {
        return reject("name must not contain whitespace");
    }
    if name.contains(RESERVED) {
        return reject("name must not contain reserved characters (: * ? \" < > |)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_names() {
        for name in ["myapp", "my_app", "my-app", "App123"] {
            assert!(
                validate_project_name(name).is_ok(),
                "should accept '{name}'"
            );
        }
    }

    #[test]
    fn rejects_empty_name() {
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("   ").is_err());
    }

    #[test]
    fn rejects_dot_specials() {
        assert!(validate_project_name(".").is_err());
        assert!(validate_project_name("..").is_err());
    }

    #[test]
    fn rejects_path_separators() {
        assert!(validate_project_name("foo/bar").is_err());
        assert!(validate_project_name("foo\\bar").is_err());
    }

    #[test]
    fn rejects_whitespace() {
        assert!(validate_project_name("my app").is_err());
        assert!(validate_project_name("my\tapp").is_err());
    }

    #[test]
    fn rejects_reserved_characters() {
        for name in ["a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b"] {
            assert!(
                validate_project_name(name).is_err(),
                "should reject '{name}'"
            );
        }
    }

    #[test]
    fn scaffold_creates_expected_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("myapp");

        scaffold(
            &dir,
            "myapp",
            "My App",
            "0.2.0",
            "a test app",
            "tester",
            "com.example.myapp",
            &["web"],
        )
        .unwrap();

        // Core files and directories are present.
        assert!(dir.join("src/main.rs").exists(), "missing src/main.rs");
        assert!(dir.join("build.rs").exists(), "missing build.rs");
        assert!(dir.join("Cargo.toml").exists(), "missing Cargo.toml");
        assert!(dir.join("Aimer.toml").exists(), "missing Aimer.toml");
        assert!(dir.join(".gitignore").exists(), "missing .gitignore");
        assert!(dir.join("README.md").exists(), "missing README.md");
        assert!(dir.join("builds/web").is_dir(), "missing builds/web");

        // Generated Cargo.toml parses and carries the right package name.
        assert_eq!(
            crate::config::parse_cargo_package_name(&dir),
            Some("myapp".to_string())
        );
        let cargo_manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        let cargo_manifest: toml::Value = toml::from_str(&cargo_manifest).unwrap();
        let crate_types = cargo_manifest["lib"]["crate-type"].as_array().unwrap();
        assert!(
            crate_types
                .iter()
                .any(|value| value.as_str() == Some("rlib"))
        );

        // aimer.toml round-trips with the collected metadata. Its name is the
        // application name; the Cargo package above keeps the project name.
        let manifest = AimerManifest::load_from(&dir).unwrap().unwrap();
        assert_eq!(manifest.package.name, "My App");
        assert_eq!(crate::config::resolve_package_name(&dir), "My App");
        assert_eq!(crate::config::resolve_cargo_package_name(&dir), "myapp");
        assert_eq!(manifest.package.version, "0.2.0");
        assert_eq!(manifest.package.description, "a test app");
        assert_eq!(manifest.package.author, "tester");
        assert_eq!(manifest.package.group, "com.example.myapp");
    }

    #[test]
    fn app_names_may_contain_spaces_but_not_path_syntax() {
        for name in ["My App", "myapp", "Café 2", "  padded  "] {
            assert!(validate_app_name(name).is_ok(), "should accept '{name}'");
        }
        for name in ["", "   ", ".", "..", "a/b", "a\\b", "a:b", "a|b", "a\tb"] {
            assert!(validate_app_name(name).is_err(), "should reject '{name}'");
        }
    }

    #[test]
    fn markup_escaping_covers_xml_significant_characters() {
        assert_eq!(
            escape_markup("Tom & <Jerry's> \"app\""),
            "Tom &amp; &lt;Jerry&apos;s&gt; &quot;app&quot;"
        );
    }

    #[test]
    fn scaffold_uses_the_app_name_for_labels_and_the_project_name_for_identifiers() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("myapp");

        scaffold(
            &dir,
            "myapp",
            "Fish & Chips",
            "0.1.0",
            "",
            "",
            "com.example.myapp",
            &["android", "linux", "macos", "web", "windows"],
        )
        .unwrap();

        let strings = fs::read_to_string(
            dir.join("builds/android/app/src/main/res/values/strings.xml"),
        )
        .unwrap();
        assert!(strings.contains(">Fish &amp; Chips<"), "{strings}");

        let desktop = fs::read_to_string(dir.join("builds/linux/myapp.desktop")).unwrap();
        assert!(desktop.contains("Name=Fish & Chips"), "{desktop}");
        assert!(desktop.contains("Exec=myapp"), "{desktop}");

        let html = fs::read_to_string(dir.join("builds/web/index.html")).unwrap();
        assert!(html.contains("<title>Fish &amp; Chips</title>"), "{html}");

        let manifest = fs::read_to_string(dir.join("builds/windows/app.manifest")).unwrap();
        assert!(manifest.contains("<description>Fish &amp; Chips</description>"));

        // Xcode keeps the Cargo package for the project and takes the display
        // name from the product at build time.
        assert!(dir.join("builds/macos/myapp.xcodeproj").is_dir());
        let plist = fs::read_to_string(dir.join("builds/macos/myapp/Info.plist")).unwrap();
        assert!(plist.contains("<string>$(PRODUCT_NAME)</string>"), "{plist}");
    }

    #[test]
    fn scaffold_cleanup_on_caller_failure_is_possible() {
        // Mirrors `execute`: a failed scaffold leaves a directory the caller can
        // remove. Here we just verify scaffolding then removing leaves nothing.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("throwaway");
        scaffold(
            &dir,
            "throwaway",
            "throwaway",
            "0.1.0",
            "",
            "",
            "com.example.throwaway",
            &[],
        )
        .unwrap();
        assert!(dir.exists());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!dir.exists());
    }
}

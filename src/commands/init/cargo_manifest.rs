//! Reading and adapting the `Cargo.toml` of an existing project for Aimer.

use std::path::Path;

use anyhow::{Context, bail};
use toml_edit::{Array, DocumentMut, Item, Table, value};

/// Crate types an Aimer app is built as: `cdylib` for Android, `staticlib` for
/// the Apple projects and `rlib` so the package stays linkable from Rust.
const CRATE_TYPES: [&str; 3] = ["cdylib", "staticlib", "rlib"];

/// The `Cargo.toml` that `aimer create` writes; the source of the `aimer`
/// dependency that `init` adds, so both commands depend on the same version.
const CARGO_TEMPLATE: &str = include_str!("../../../templates/Cargo.toml.template");

/// What `init` reads from the existing package to pre-fill its questions.
pub(super) struct CargoPackage {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
}

/// Parse `<dir>/Cargo.toml`, requiring a `[package]` to initialize.
pub(super) fn read(dir: &Path) -> anyhow::Result<(DocumentMut, CargoPackage)> {
    let path = dir.join("Cargo.toml");
    if !path.is_file() {
        bail!(
            "no Cargo.toml found in {} — run `aimer init` from a Cargo project root, or use `aimer create` for a new project",
            dir.display()
        );
    }
    let contents =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let document: DocumentMut = contents
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;

    let Some(package) = document.get("package").and_then(Item::as_table_like) else {
        bail!(
            "{} has no [package] table (virtual workspace?) — run `aimer init` inside a member package",
            path.display()
        );
    };
    let text = |key: &str| {
        package
            .get(key)
            .and_then(Item::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let Some(name) = text("name") else {
        bail!("{} has no [package].name", path.display());
    };
    let author = package
        .get("authors")
        .and_then(Item::as_array)
        .and_then(|authors| authors.iter().find_map(|author| author.as_str()))
        .map(str::to_string);

    let package = CargoPackage {
        name,
        version: text("version"),
        description: text("description"),
        author,
    };
    Ok((document, package))
}

/// Make the package buildable as an Aimer app: an Aimer-capable `[lib]` and the
/// `aimer` dependency. Existing settings are kept and only gaps are filled.
///
/// Returns a line for every change made, empty when the manifest already fit.
pub(super) fn adapt(document: &mut DocumentMut, dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut changes = Vec::new();
    adapt_lib(document, dir, &mut changes)?;
    adapt_dependency(document, &mut changes)?;
    Ok(changes)
}

fn adapt_lib(
    document: &mut DocumentMut,
    dir: &Path,
    changes: &mut Vec<String>,
) -> anyhow::Result<()> {
    let lib = document
        .entry("lib")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_like_mut()
        .context("[lib] in Cargo.toml is not a table")?;

    if lib.get("path").is_none() && !dir.join("src/lib.rs").is_file() {
        // Aimer builds the app as a library; an app that only has `main.rs`
        // exposes that file as the library too, exactly as `aimer create` does.
        if !dir.join("src/main.rs").is_file() {
            bail!("found neither src/lib.rs nor src/main.rs to build the Aimer library from");
        }
        lib.insert("path", value("src/main.rs"));
        changes.push("[lib] path = \"src/main.rs\"".to_string());
    }

    let mut missing = Vec::new();
    match lib.get_mut("crate-type").and_then(Item::as_array_mut) {
        Some(existing) => {
            for crate_type in CRATE_TYPES {
                if !existing
                    .iter()
                    .any(|item| item.as_str() == Some(crate_type))
                {
                    existing.push(crate_type);
                    missing.push(crate_type);
                }
            }
        }
        None => {
            lib.insert("crate-type", value(Array::from_iter(CRATE_TYPES)));
            missing.extend(CRATE_TYPES);
        }
    }
    if !missing.is_empty() {
        changes.push(format!("[lib] crate-type += {}", missing.join(", ")));
    }
    Ok(())
}

fn adapt_dependency(document: &mut DocumentMut, changes: &mut Vec<String>) -> anyhow::Result<()> {
    let dependencies = document
        .entry("dependencies")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_like_mut()
        .context("[dependencies] in Cargo.toml is not a table")?;
    if dependencies.contains_key("aimer") {
        return Ok(());
    }

    let template: DocumentMut = CARGO_TEMPLATE
        .parse()
        .context("parsing the bundled Cargo.toml template")?;
    let aimer = template["dependencies"]["aimer"].clone();
    dependencies.insert("aimer", aimer);
    changes.push("[dependencies] aimer".to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(cargo: &str, files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "fn main() {}\n").unwrap();
        }
        std::fs::write(dir.path().join("Cargo.toml"), cargo).unwrap();
        dir
    }

    #[test]
    fn reads_the_package_metadata() {
        let dir = project(
            "[package]\nname = \"demo\"\nversion = \"1.2.3\"\ndescription = \"d\"\nauthors = [\"a\", \"b\"]\n",
            &[],
        );
        let (_, package) = read(dir.path()).unwrap();
        assert_eq!(package.name, "demo");
        assert_eq!(package.version.as_deref(), Some("1.2.3"));
        assert_eq!(package.description.as_deref(), Some("d"));
        assert_eq!(package.author.as_deref(), Some("a"));
    }

    #[test]
    fn inherited_version_is_not_a_default() {
        let dir = project(
            "[package]\nname = \"demo\"\nversion.workspace = true\n",
            &[],
        );
        assert!(read(dir.path()).unwrap().1.version.is_none());
    }

    #[test]
    fn rejects_a_missing_or_virtual_manifest() {
        let empty = tempfile::tempdir().unwrap();
        assert!(read(empty.path()).is_err());
        let workspace = project("[workspace]\nmembers = []\n", &[]);
        assert!(
            read(workspace.path())
                .err()
                .unwrap()
                .to_string()
                .contains("no [package]")
        );
    }

    #[test]
    fn adapts_a_binary_package() {
        let dir = project(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            &["src/main.rs"],
        );
        let (mut document, _) = read(dir.path()).unwrap();
        let changes = adapt(&mut document, dir.path()).unwrap();
        assert_eq!(changes.len(), 3, "{changes:?}");

        let parsed: toml::Value = toml::from_str(&document.to_string()).unwrap();
        assert_eq!(parsed["lib"]["path"].as_str(), Some("src/main.rs"));
        let crate_types: Vec<_> = parsed["lib"]["crate-type"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.as_str())
            .collect();
        assert_eq!(crate_types, CRATE_TYPES);
        assert!(parsed["dependencies"]["aimer"]["git"].is_str());
    }

    #[test]
    fn keeps_existing_settings_and_is_idempotent() {
        let dir = project(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[lib]\ncrate-type = [\"rlib\", \"dylib\"]\n\n[dependencies]\naimer = \"1\"\nserde = \"1\"\n",
            &["src/lib.rs"],
        );
        let (mut document, _) = read(dir.path()).unwrap();
        adapt(&mut document, dir.path()).unwrap();

        let parsed: toml::Value = toml::from_str(&document.to_string()).unwrap();
        assert!(
            parsed["lib"].get("path").is_none(),
            "src/lib.rs is implicit"
        );
        let crate_types: Vec<_> = parsed["lib"]["crate-type"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.as_str())
            .collect();
        assert_eq!(crate_types, ["rlib", "dylib", "cdylib", "staticlib"]);
        assert_eq!(parsed["dependencies"]["aimer"].as_str(), Some("1"));
        assert_eq!(parsed["dependencies"]["serde"].as_str(), Some("1"));

        assert!(adapt(&mut document, dir.path()).unwrap().is_empty());
    }

    #[test]
    fn needs_a_source_entry() {
        let dir = project("[package]\nname = \"demo\"\nversion = \"0.1.0\"\n", &[]);
        let (mut document, _) = read(dir.path()).unwrap();
        assert!(adapt(&mut document, dir.path()).is_err());
    }
}

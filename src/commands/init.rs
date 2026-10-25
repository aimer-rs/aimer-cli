mod cargo_manifest;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use toml_edit::DocumentMut;

use super::create::prompt::{self, Answers, Defaults};
use super::create::{GITIGNORE_TEMPLATE, scaffold_platforms};
use crate::config::AimerManifest;

/// Turn an existing Cargo package into an Aimer project.
///
/// Asks the same questions as `aimer create` and generates the same platform
/// scaffolds and `Aimer.toml`, but adapts the project in place: the package's
/// sources and README are never touched, `Cargo.toml` and `.gitignore` are only
/// extended, and nothing that already exists is replaced. No `build.rs` is
/// written: `aimer build` and `aimer run` pass the iOS link flags themselves.
pub fn execute(path: Option<&Path>) -> anyhow::Result<()> {
    let dir = path.unwrap_or(Path::new("."));
    let dir = dir
        .canonicalize()
        .with_context(|| format!("resolving project directory {}", dir.display()))?;

    let (document, package) = cargo_manifest::read(&dir)?;
    ensure_not_initialized(&dir)?;

    prompt::install_render_config();
    println!(
        "Initializing Aimer in '{}' ({})",
        package.name,
        dir.display()
    );

    let answers = prompt::collect(&Defaults {
        app_name: &package.name,
        version: package.version.as_deref().unwrap_or("0.1.0"),
        description: package.description.as_deref().unwrap_or_default(),
        author: package.author.as_deref().unwrap_or_default(),
    });
    if !prompt::confirm(&package.name, &answers) {
        println!("Aborted.");
        return Ok(());
    }

    initialize(&dir, &package.name, document, &answers)
        .with_context(|| format!("failed to initialize '{}'", package.name))?;

    println!("Project '{}' initialized successfully.", package.name);
    Ok(())
}

/// Apply the answers to the package in `dir`.
///
/// Fails before writing anything when the project cannot be initialized, and
/// undoes what it created if a later write fails.
fn initialize(
    dir: &Path,
    name: &str,
    mut document: DocumentMut,
    answers: &Answers,
) -> anyhow::Result<()> {
    ensure_not_initialized(dir)?;
    ensure_targets_free(dir, &answers.targets)?;
    let changes = cargo_manifest::adapt(&mut document, dir)?;

    let mut undo = Undo::default();
    let result = write_project(dir, name, &document, &changes, answers, &mut undo);
    if result.is_err() {
        undo.run();
    }
    result
}

fn write_project(
    dir: &Path,
    name: &str,
    document: &DocumentMut,
    changes: &[String],
    answers: &Answers,
    undo: &mut Undo,
) -> anyhow::Result<()> {
    for target in &answers.targets {
        undo.created.push(dir.join("builds").join(target));
    }
    scaffold_platforms(
        dir,
        name,
        &answers.app_name,
        &answers.group,
        &answers.targets,
    );
    for target in &answers.targets {
        println!("  ✔ {target} scaffold created");
    }

    if !changes.is_empty() {
        let path = dir.join("Cargo.toml");
        let original = fs::read_to_string(&path).context("reading Cargo.toml")?;
        undo.restore.push((path.clone(), original));
        fs::write(&path, document.to_string()).context("writing Cargo.toml")?;
        for change in changes {
            println!("  ✔ Cargo.toml: {change}");
        }
    }

    let gitignore = dir.join(".gitignore");
    let existing = match fs::read_to_string(&gitignore) {
        Ok(contents) => Some(contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("reading .gitignore"),
    };
    if let Some(merged) =
        merge_gitignore(existing.as_deref().unwrap_or_default(), GITIGNORE_TEMPLATE)
    {
        match existing {
            Some(original) => undo.restore.push((gitignore.clone(), original)),
            None => undo.created.push(gitignore.clone()),
        }
        fs::write(&gitignore, merged).context("writing .gitignore")?;
        println!("  ✔ .gitignore updated");
    }

    let manifest = dir.join(crate::config::MANIFEST_FILE);
    AimerManifest::new(
        &answers.app_name,
        &answers.version,
        &answers.description,
        &answers.author,
        &answers.group,
    )
    .write_to(dir)
    .context("writing Aimer.toml")?;
    undo.created.push(manifest);
    println!("  ✔ Aimer.toml created");

    Ok(())
}

/// A project that already has an Aimer manifest is initialized; upgrading its
/// scaffolds is `aimer migrate`'s job.
fn ensure_not_initialized(dir: &Path) -> anyhow::Result<()> {
    for name in [crate::config::MANIFEST_FILE, "aimer.toml"] {
        if dir.join(name).exists() {
            bail!(
                "{} already exists — this is already an Aimer project; use `aimer migrate` to update its platform scaffolds",
                dir.join(name).display()
            );
        }
    }
    Ok(())
}

/// Refuse to overwrite a platform folder the user already has.
fn ensure_targets_free(dir: &Path, targets: &[&str]) -> anyhow::Result<()> {
    for target in targets {
        let platform_dir = dir.join("builds").join(target);
        if platform_dir.exists() {
            bail!(
                "builds/{target} already exists — move it away, or use `aimer migrate {target}` to regenerate it"
            );
        }
    }
    Ok(())
}

/// The ignore rules of the Aimer template that are missing from `existing`.
///
/// Only the `builds/` rules are taken over: they hide generated platform
/// output, whereas rules like `Cargo.lock` are policy the project already
/// decided. Returns the full new contents, or `None` when nothing is missing.
fn merge_gitignore(existing: &str, template: &str) -> Option<String> {
    let known: Vec<&str> = existing.lines().map(str::trim).collect();
    let missing: Vec<&str> = template
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("builds/") && !known.contains(line))
        .collect();
    if missing.is_empty() {
        return None;
    }

    let mut merged = existing.to_string();
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    if !merged.is_empty() {
        merged.push('\n');
    }
    merged.push_str("# aimer platform builds\n");
    for line in missing {
        merged.push_str(line);
        merged.push('\n');
    }
    Some(merged)
}

/// What `initialize` has to take back when it fails half-way.
#[derive(Default)]
struct Undo {
    /// Files and directories that did not exist before.
    created: Vec<PathBuf>,
    /// Files that existed, with their original contents.
    restore: Vec<(PathBuf, String)>,
}

impl Undo {
    fn run(self) {
        for path in self.created.iter().rev() {
            if path.is_dir() {
                let _ = fs::remove_dir_all(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
        for (path, contents) in self.restore {
            let _ = fs::write(path, contents);
        }
    }
}

#[cfg(test)]
mod tests;

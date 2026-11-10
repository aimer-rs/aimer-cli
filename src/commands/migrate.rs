mod merge;

use std::path::Path;

use anyhow::{Context, bail};

use crate::commands::create;
use crate::config::AimerManifest;

/// A scaffolder writes `builds/<platform>/` below the directory it is given.
type Scaffolder = fn(&Path, &str, &str, &str);

/// Every platform `migrate` knows, in the order `all` walks them.
const PLATFORMS: &[&str] = &["web", "macos", "ios", "android", "windows", "linux"];

fn scaffolder(platform: &str) -> Option<Scaffolder> {
    Some(match platform {
        "web" => create::web::create,
        "macos" => create::macos::create,
        "ios" => create::ios::create,
        "android" => create::android::create,
        "windows" => create::window::create,
        "linux" => create::linux::create,
        _ => return None,
    })
}

/// What every scaffolder needs, plus whether to overwrite user-edited files.
struct Project<'a> {
    dir: &'a Path,
    name: &'a str,
    app_name: &'a str,
    group: &'a str,
    force: bool,
}

/// Migrate platform build scaffolds to the latest version.
///
/// Reads `Aimer.toml` to get the application name and `group`, and `Cargo.toml`
/// for the package the scaffold identifiers are derived from, then brings the
/// build scaffold for the requested target (or all targets) up to the
/// templates bundled in the current CLI version.
///
/// The scaffold is merged into `builds/<platform>/`, never deleted and rebuilt:
/// files the project lacks are added, generated code is refreshed, and files
/// that may hold the user's own settings (`Info.plist`, `project.pbxproj`,
/// `AndroidManifest.xml`, icons, ...) are kept as they are. `force` replaces
/// those too.
pub fn execute(target: String, force: bool) -> anyhow::Result<()> {
    execute_in(target, Path::new("."), force)
}

/// Internal implementation that accepts an explicit directory for testability.
fn execute_in(target: String, dir: &Path, force: bool) -> anyhow::Result<()> {
    let manifest = AimerManifest::load_from(dir)
        .context("failed to read aimer.toml")?
        .context("no Aimer.toml found — run this command from an Aimer project root")?;

    // Resolve to an absolute path. The `create::*` scaffolders derive the
    // project name from `dir.file_name()`, but a relative `.` (the usual case
    // when running from the project root) has no file name and would otherwise
    // make scaffolding fail, so the target folders never get generated.
    let canonical = dir
        .canonicalize()
        .with_context(|| format!("resolving project directory {}", dir.display()))?;
    let dir = canonical.as_path();

    // Identifiers (Xcode project, desktop entry, Trunk bin) follow the Cargo
    // package; the manifest name is the application's display name.
    let name = crate::config::resolve_cargo_package_name(dir);
    let app_name = manifest.package.name.clone();
    let group = if manifest.package.group.is_empty() {
        "com.example.app".to_string()
    } else {
        manifest.package.group.clone()
    };
    let project = Project {
        dir,
        name: &name,
        app_name: &app_name,
        group: &group,
        force,
    };

    match target.as_str() {
        "all" => {
            for platform in PLATFORMS {
                migrate_platform(&project, platform)?;
            }
        }
        platform if scaffolder(platform).is_some() => migrate_platform(&project, platform)?,
        other => {
            bail!(
                "unknown target '{other}'. Valid targets: macos, windows, linux, android, ios, web, all"
            )
        }
    }

    println!("Migration complete for '{target}'. Project: {app_name} (group: {group})");
    Ok(())
}

/// Migrate one platform scaffold.
///
/// The latest scaffold is generated into a scratch directory and merged into
/// `builds/<platform>/`, so nothing the project already has is deleted: build
/// artifacts (`pkg/`, `dist/`), signing material and hand edits all survive.
fn migrate_platform(project: &Project<'_>, platform: &str) -> anyhow::Result<()> {
    let create_fn = scaffolder(platform).with_context(|| format!("unknown platform {platform}"))?;

    let staging = tempfile::tempdir().context("creating a scratch directory")?;
    create_fn(
        staging.path(),
        project.name,
        project.app_name,
        project.group,
    );

    let relative = Path::new("builds").join(platform);
    let report = merge::merge(
        &staging.path().join(&relative),
        &project.dir.join(&relative),
        project.force,
    )?;

    println!(
        "  ✔ {platform} scaffold migrated ({} added, {} refreshed, {} kept)",
        report.added.len(),
        report.refreshed.len(),
        report.kept.len()
    );
    if !report.kept.is_empty() {
        println!(
            "    kept your version of these files, which differ from the latest template (use --force to replace them):"
        );
        for file in &report.kept {
            println!("      {}", relative.join(file).display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

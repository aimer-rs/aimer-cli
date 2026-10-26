use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow, bail};
use serde::Deserialize;

use crate::config::{AimerManifest, MANIFEST_FILE};

const LOWERCASE_MANIFEST_FILE: &str = "aimer.toml";

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct AimerProject {
    pub(crate) root: PathBuf,
    pub(crate) cargo_package: String,
    pub(crate) aimer_package: String,
}

pub(crate) struct ProjectDirectoryGuard {
    previous: PathBuf,
}

impl AimerProject {
    /// Enter the selected project's directory for existing build and run steps.
    pub(crate) fn enter(&self) -> anyhow::Result<ProjectDirectoryGuard> {
        let previous = std::env::current_dir().context("reading current directory")?;
        std::env::set_current_dir(&self.root)
            .with_context(|| format!("entering Aimer project '{}'", self.root.display()))?;
        Ok(ProjectDirectoryGuard { previous })
    }
}

impl Drop for ProjectDirectoryGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.previous);
    }
}

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    manifest_path: PathBuf,
}

/// Select an Aimer app in the current Cargo workspace.
///
/// A project containing the current directory wins when no package was named.
/// Otherwise a single Aimer package is selected automatically, while multiple
/// candidates require `--package`.
pub(crate) fn select_aimer_project(package: Option<&str>) -> anyhow::Result<AimerProject> {
    let current_dir = std::env::current_dir().context("reading current directory")?;
    select_aimer_project_from(&current_dir, package)
}

fn select_aimer_project_from(
    current_dir: &Path,
    package: Option<&str>,
) -> anyhow::Result<AimerProject> {
    let workspace_manifest = workspace_manifest(current_dir)?;
    let metadata = cargo_metadata(&workspace_manifest)?;
    let mut projects = metadata
        .packages
        .into_iter()
        .map(project_from_package)
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    projects.sort_by(|left, right| left.cargo_package.cmp(&right.cargo_package));

    if let Some(package) = package {
        let matches = projects
            .iter()
            .filter(|project| project.cargo_package == package || project.aimer_package == package)
            .cloned()
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [project] => Ok(project.clone()),
            [] => bail!(
                "no Aimer project named '{package}' in Cargo workspace '{}'; available Aimer packages: {}",
                workspace_manifest.display(),
                package_list(&projects)
            ),
            _ => bail!(
                "package name '{package}' matches multiple Aimer projects; select by Cargo package name: {}",
                matches
                    .iter()
                    .map(|project| project.cargo_package.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
    }

    let current_project = projects
        .iter()
        .filter(|project| current_dir.starts_with(&project.root))
        .max_by_key(|project| project.root.components().count());
    if let Some(project) = current_project {
        return Ok(project.clone());
    }

    match projects.as_slice() {
        [project] => Ok(project.clone()),
        [] => bail!(
            "no Aimer.toml or aimer.toml found in Cargo workspace '{}'",
            workspace_manifest.display()
        ),
        _ => bail!(
            "multiple Aimer projects found in Cargo workspace '{}':\n\n{}\n\nselect one with --package <NAME> (-p <NAME>)",
            workspace_manifest.display(),
            package_list(&projects)
        ),
    }
}

fn workspace_manifest(current_dir: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .current_dir(current_dir)
        .output()
        .context("starting cargo locate-project")?;
    if !output.status.success() {
        bail!(
            "could not find a Cargo workspace from '{}'; run this command inside a Cargo workspace",
            current_dir.display()
        );
    }
    let path = PathBuf::from(String::from_utf8(output.stdout)?.trim());
    if !path.is_file() {
        bail!(
            "Cargo reported a missing workspace manifest at '{}';",
            path.display()
        );
    }
    Ok(path)
}

fn cargo_metadata(manifest: &Path) -> anyhow::Result<CargoMetadata> {
    let output = Command::new("cargo")
        .arg("metadata")
        .args(["--no-deps", "--format-version", "1", "--manifest-path"])
        .arg(manifest)
        .output()
        .with_context(|| {
            format!(
                "reading Cargo workspace metadata from '{}'",
                manifest.display()
            )
        })?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed for '{}': {}",
            manifest.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "parsing Cargo workspace metadata from '{}'",
            manifest.display()
        )
    })
}

fn project_from_package(package: CargoPackage) -> anyhow::Result<Option<AimerProject>> {
    let Some(root) = package.manifest_path.parent() else {
        return Ok(None);
    };
    if !has_aimer_manifest(root) {
        return Ok(None);
    }
    let manifest = AimerManifest::load_from(root)?
        .ok_or_else(|| anyhow!("Aimer manifest disappeared from '{}'", root.display()))?;
    Ok(Some(AimerProject {
        root: root.to_path_buf(),
        cargo_package: package.name,
        aimer_package: manifest.package.name,
    }))
}

fn has_aimer_manifest(root: &Path) -> bool {
    root.join(MANIFEST_FILE).is_file() || root.join(LOWERCASE_MANIFEST_FILE).is_file()
}

fn package_list(projects: &[AimerProject]) -> String {
    projects
        .iter()
        .map(|project| {
            if project.cargo_package == project.aimer_package {
                format!("{} ({})", project.cargo_package, project.root.display())
            } else {
                format!(
                    "{} (Aimer name: {}; {})",
                    project.cargo_package,
                    project.aimer_package,
                    project.root.display()
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_projects_require_package_when_selection_is_ambiguous() {
        let temp = tempfile::tempdir().unwrap();
        let root = &temp.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("apps/first")).unwrap();
        std::fs::create_dir_all(root.join("apps/second")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = ['apps/first', 'apps/second']\nresolver = '2'\n",
        )
        .unwrap();
        for (path, name) in [("first", "first-app"), ("second", "second-app")] {
            let app = root.join("apps").join(path);
            std::fs::write(
                app.join("Cargo.toml"),
                format!("[package]\nname = '{name}'\nversion = '0.1.0'\nedition = '2021'\n"),
            )
            .unwrap();
            std::fs::create_dir_all(app.join("src")).unwrap();
            std::fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
            std::fs::write(
                app.join("aimer.toml"),
                format!("[package]\nname = '{name}'\n"),
            )
            .unwrap();
        }

        let selected = select_aimer_project_from(root, Some("second-app")).unwrap();
        assert_eq!(selected.cargo_package, "second-app");
        // Compare canonicalized forms on both sides: `cargo metadata` reports
        // `manifest_path` without Windows' `\\?\` verbatim prefix, even when the
        // input root already carries it, so a raw comparison can mismatch there.
        assert_eq!(
            selected.root.canonicalize().unwrap(),
            root.join("apps/second").canonicalize().unwrap()
        );

        let result = select_aimer_project_from(root, None);

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("multiple Aimer projects")
        );
    }
}

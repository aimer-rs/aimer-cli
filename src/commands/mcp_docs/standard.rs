//! Offline documentation from rustup's `rust-docs-json` component.

use super::*;
use tokio::process::Command;

pub(super) fn is_toolchain_crate(name: &str) -> bool {
    matches!(name, "std" | "core" | "alloc" | "proc_macro" | "test")
}

pub(super) async fn load(
    project: &Path,
    crate_name: &str,
    toolchain: Option<&str>,
) -> anyhow::Result<(RustdocCrate, String, String)> {
    if let Some(toolchain) = toolchain {
        validate_toolchain(toolchain)?;
    }
    let (mut sysroot, mut version) = compiler_info(project, toolchain).await?;
    let mut json_dir = sysroot.join("share/doc/rust/json");
    // Stable toolchains generally do not ship JSON docs. Use an installed nightly
    // only when no toolchain was explicitly requested, and report its real version.
    if toolchain.is_none() && !json_dir.join(format!("{crate_name}.json")).is_file() {
        if let Ok(info) = compiler_info(project, Some("nightly")).await {
            (sysroot, version) = info;
            json_dir = sysroot.join("share/doc/rust/json");
        }
    }
    if !json_dir.join(format!("{crate_name}.json")).is_file() {
        bail!(
            "{crate_name} JSON documentation is missing in {}. Install it with `rustup component add --toolchain {} rust-docs-json`, then retry with that toolchain as version",
            json_dir.display(),
            toolchain.unwrap_or("nightly")
        );
    }
    let crate_name = crate_name.to_owned();
    let load_name = crate_name.clone();
    let load_version = version.clone();
    let rustdoc =
        tokio::task::spawn_blocking(move || load_indices(&json_dir, &load_name, &load_version))
            .await
            .context("loading toolchain documentation")??;
    let channel = if version.contains("nightly") {
        "nightly"
    } else if version.contains("beta") {
        "beta"
    } else {
        &version
    };
    let docs_url = format!("https://doc.rust-lang.org/{channel}/{crate_name}/");
    Ok((rustdoc, version, docs_url))
}

fn validate_toolchain(toolchain: &str) -> anyhow::Result<()> {
    if toolchain.is_empty()
        || !toolchain.as_bytes()[0].is_ascii_alphanumeric()
        || !toolchain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || toolchain.contains("..")
    {
        bail!("version for a toolchain crate must be a Rust toolchain such as nightly or 1.99.0");
    }
    Ok(())
}

async fn compiler_info(
    project: &Path,
    toolchain: Option<&str>,
) -> anyhow::Result<(PathBuf, String)> {
    let (sysroot, stdout) = tokio::try_join!(
        compiler_output(project, toolchain, &["--print", "sysroot"]),
        compiler_output(project, toolchain, &["--version"]),
    )?;
    let version = stdout
        .trim()
        .strip_prefix("rustc ")
        .and_then(|line| line.split_whitespace().next())
        .context("Rust toolchain did not report a compiler version")?
        .to_owned();
    let sysroot = sysroot.trim();
    if sysroot.is_empty() {
        bail!("Rust toolchain did not report its sysroot");
    }
    Ok((PathBuf::from(sysroot), version))
}

async fn compiler_output(
    project: &Path,
    toolchain: Option<&str>,
    args: &[&str],
) -> anyhow::Result<String> {
    let mut command = match toolchain {
        Some(toolchain) => {
            let mut command = Command::new("rustup");
            command.args(["run", toolchain, "rustc"]);
            command
        }
        None => Command::new("rustc"),
    };
    command.current_dir(project).args(args).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .context("timed out resolving the Rust toolchain")?
        .context("resolving the Rust toolchain")?;
    if !output.status.success() {
        bail!(
            "cannot resolve Rust toolchain: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("Rust toolchain output is not UTF-8")
}

fn read_index(directory: &Path, name: &str, version: &str) -> anyhow::Result<RustdocCrate> {
    let path = directory.join(format!("{name}.json"));
    let file = File::open(&path).with_context(|| format!("reading {}", path.display()))?;
    if file.metadata()?.len() > MAX_UNCOMPRESSED_BYTES {
        bail!("toolchain documentation exceeds the {MAX_UNCOMPRESSED_BYTES}-byte limit");
    }
    let mut bytes = Vec::new();
    file.take(MAX_UNCOMPRESSED_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_UNCOMPRESSED_BYTES {
        bail!("toolchain documentation exceeds the {MAX_UNCOMPRESSED_BYTES}-byte limit");
    }
    let index: RustdocCrate =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    if index.format_version == 0
        || index.index.is_empty()
        || index.root_item()?.name.as_deref() != Some(name)
    {
        bail!("invalid toolchain documentation in {}", path.display());
    }
    if index
        .crate_version
        .as_deref()
        .and_then(|v| v.split_whitespace().next())
        .is_some_and(|actual| actual != version)
    {
        bail!(
            "toolchain documentation in {} does not match rustc {version}",
            path.display()
        );
    }
    Ok(index)
}

fn load_indices(directory: &Path, crate_name: &str, version: &str) -> anyhow::Result<RustdocCrate> {
    let mut crates = vec![(
        crate_name.to_owned(),
        read_index(directory, crate_name, version)?,
    )];
    // std publicly re-exports core and alloc; their definitions are absent from
    // std's JSON index. Match external path summaries to the defining crate.
    for dependency in ["core", "alloc"] {
        if dependency != crate_name
            && crates[0]
                .1
                .paths
                .values()
                .any(|path| path.path.first().is_some_and(|name| name == dependency))
        {
            crates.push((
                dependency.to_owned(),
                read_index(directory, dependency, version)?,
            ));
        }
    }
    merge_indices(crates)
}

fn merge_indices(crates: Vec<(String, RustdocCrate)>) -> anyhow::Result<RustdocCrate> {
    let mut definitions = HashMap::new();
    for (name, rustdoc) in &crates {
        let local_id = rustdoc.root_item()?.crate_id;
        for (id, path) in &rustdoc.paths {
            if rustdoc
                .index
                .get(id)
                .is_some_and(|item| item.crate_id == local_id)
            {
                definitions.insert(
                    (path.path.clone(), path.kind.clone()),
                    format!("{name}:{id}"),
                );
            }
        }
    }
    let mut merged = RustdocCrate {
        root: format!("{}:{}", crates[0].0, crates[0].1.root),
        format_version: crates[0].1.format_version,
        crate_version: crates[0].1.crate_version.clone(),
        index: HashMap::new(),
        paths: HashMap::new(),
    };
    for (name, rustdoc) in crates {
        let local_id = rustdoc.root_item()?.crate_id;
        let local_ids = rustdoc
            .index
            .iter()
            .filter(|(_, item)| item.crate_id == local_id)
            .map(|(id, _)| id.clone())
            .collect::<std::collections::HashSet<_>>();
        let remap = |id: &str| {
            rustdoc
                .paths
                .get(id)
                .filter(|_| !local_ids.contains(id))
                .and_then(|path| definitions.get(&(path.path.clone(), path.kind.clone())))
                .cloned()
                .unwrap_or_else(|| format!("{name}:{id}"))
        };
        for (id, mut item) in rustdoc.index {
            remap_ids(&mut item.inner, &remap);
            item.crate_id = 0;
            merged.index.insert(format!("{name}:{id}"), item);
        }
    }
    Ok(merged)
}

fn remap_ids(value: &mut serde_json::Value, remap: &impl Fn(&str) -> String) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                match key.as_str() {
                    "id" => {
                        if let Some(id) = id_value(value) {
                            *value = remap(&id).into();
                        }
                    }
                    "items" | "impls" | "variants" | "fields" | "tuple" => {
                        if let Some(ids) = value.as_array_mut() {
                            for value in ids {
                                if let Some(id) = id_value(value) {
                                    *value = remap(&id).into();
                                }
                            }
                        } else {
                            remap_ids(value, remap);
                        }
                    }
                    _ => remap_ids(value, remap),
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                remap_ids(value, remap);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;

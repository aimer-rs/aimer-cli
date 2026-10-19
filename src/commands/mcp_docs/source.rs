use super::*;

pub(super) fn resolve_locked_version(
    project_root: &Path,
    crate_name: &str,
) -> anyhow::Result<String> {
    let path = project_root
        .ancestors()
        .map(|directory| directory.join("Cargo.lock"))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            anyhow!(
                "no Cargo.lock was found at or above {}; pass an explicit version",
                project_root.display()
            )
        })?;
    let contents = fs::read_to_string(&path).with_context(|| {
        format!(
            "version was omitted and {} could not be read; pass an explicit version",
            path.display()
        )
    })?;
    let lockfile: Lockfile =
        toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    let mut versions = lockfile
        .package
        .into_iter()
        .filter(|package| package.name == crate_name)
        .filter(|package| package.source.as_deref().is_some_and(is_crates_io_registry))
        .map(|package| package.version)
        .collect::<Vec<_>>();
    versions.sort();
    versions.dedup();
    match versions.as_slice() {
        [version] => Ok(version.clone()),
        [] => bail!(
            "{crate_name} is not a crates.io dependency in this project's Cargo.lock; pass an explicit version"
        ),
        _ => bail!(
            "multiple locked versions of {crate_name} were found ({}); pass an explicit version",
            versions.join(", ")
        ),
    }
}

pub(super) fn is_crates_io_registry(source: &str) -> bool {
    source.starts_with("registry+")
        && (source.ends_with("crates.io-index") || source.contains("index.crates.io"))
}

pub(super) fn validate_crate_name(crate_name: &str) -> anyhow::Result<()> {
    if crate_name.is_empty()
        || !crate_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("crate_name must contain only ASCII letters, digits, hyphens, or underscores");
    }
    Ok(())
}

pub(super) fn validate_search_query(query: &str) -> anyhow::Result<()> {
    if query.is_empty() {
        bail!("query must not be empty");
    }
    if query.len() > 256 {
        bail!("query must be at most 256 bytes");
    }
    if query.chars().any(char::is_control) {
        bail!("query must not contain control characters");
    }
    Ok(())
}

pub(super) fn short_summary(documentation: &str) -> Option<String> {
    let normalized = documentation
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return None;
    }
    let mut characters = normalized.chars();
    let summary = characters.by_ref().take(280).collect::<String>();
    Some(if characters.next().is_some() {
        format!("{summary}…")
    } else {
        summary
    })
}

pub(super) fn validate_version(version: &str) -> anyhow::Result<()> {
    if version.is_empty()
        || !version.as_bytes()[0].is_ascii_digit()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
        || version.contains("..")
    {
        bail!("version must be an exact semantic version such as 1.2.3");
    }
    Ok(())
}

pub(super) fn docs_json_url(crate_name: &str, version: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(DOCS_RS_JSON_URL)?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("could not construct docs.rs URL"))?
        .push(crate_name)
        .push(version)
        .push("json.gz");
    Ok(url)
}

pub(super) fn crate_docs_url(
    crate_name: &str,
    version: &str,
    crate_root: &str,
) -> anyhow::Result<String> {
    let mut url = Url::parse("https://docs.rs/")?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("could not construct docs.rs URL"))?
        .pop_if_empty()
        .push(crate_name)
        .push(version)
        .push(crate_root);
    Ok(url.to_string())
}

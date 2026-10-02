//! Shared read-only docs.rs access for the Aimer MCP server and CLI.

use std::collections::HashMap;
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, anyhow, bail};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use reqwest::{Client, StatusCode, Url};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const DOCS_RS_JSON_URL: &str = "https://docs.rs/crate";
const MAX_COMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GetCrateDocsArguments {
    /// Cargo package name, for example `serde`.
    pub(crate) crate_name: String,
    /// Rust item path relative to the crate root, for example `de::Deserialize`.
    /// If omitted, the crate-level documentation is returned.
    #[serde(default)]
    pub(crate) item_path: Option<String>,
    /// Exact crate version. If omitted, Aimer resolves it from the attached project's Cargo.lock.
    #[serde(default)]
    pub(crate) version: Option<String>,
    /// Refresh the cached crate index from docs.rs.
    #[serde(default)]
    pub(crate) refresh: bool,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SearchCrateDocsArguments {
    /// Cargo package name, for example `serde`.
    pub(crate) crate_name: String,
    /// Case-insensitive text to find in public item names, paths, and docs.
    pub(crate) query: String,
    /// Exact crate version. If omitted, Aimer resolves it from the attached project's Cargo.lock.
    #[serde(default)]
    pub(crate) version: Option<String>,
    /// Maximum number of results to return (1–100; defaults to 20).
    #[serde(default)]
    pub(crate) limit: Option<usize>,
    /// Refresh the cached crate index from docs.rs.
    #[serde(default)]
    pub(crate) refresh: bool,
}

#[derive(Clone)]
pub struct DocsRsClient {
    project_root: PathBuf,
    cache_root: PathBuf,
    http: Client,
}

impl DocsRsClient {
    pub fn new(project_root: PathBuf) -> anyhow::Result<Self> {
        let cache_root = dirs::home_dir()
            .context("cannot determine the user's home directory")?
            .join(".aimer")
            .join("docs")
            .join("docs.rs");
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .context("creating the docs.rs HTTP client")?;

        Ok(Self {
            project_root,
            cache_root,
            http,
        })
    }

    pub async fn get_document(
        &self,
        arguments: GetCrateDocsArguments,
    ) -> anyhow::Result<CrateDocumentation> {
        let crate_name = arguments.crate_name.trim().to_owned();
        validate_crate_name(&crate_name)?;
        let version = match arguments.version.as_deref() {
            Some(version) => {
                validate_version(version)?;
                version.to_owned()
            }
            None => resolve_locked_version(&self.project_root, &crate_name)?,
        };

        let cache_path = self.cache_path(&crate_name, &version);
        let zstd_cache_path = cache_path.with_file_name("default.json.zst");
        let (rustdoc, source, cache_warning) = self
            .load_crate_index(
                &crate_name,
                &version,
                &cache_path,
                &zstd_cache_path,
                arguments.refresh,
            )
            .await?;
        let selected = rustdoc.select_item(arguments.item_path.as_deref())?;
        let root = rustdoc.root_item()?;
        let crate_root = root
            .name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&crate_name);
        let docs_url = crate_docs_url(&crate_name, &version, crate_root)?;

        Ok(CrateDocumentation {
            crate_name,
            version,
            item_path: selected.path,
            item_kind: selected.kind,
            documentation: selected.docs,
            trait_details: selected.trait_details,
            docs_url,
            source,
            cache_warning,
        })
    }

    pub async fn search_documents(
        &self,
        arguments: SearchCrateDocsArguments,
    ) -> anyhow::Result<CrateSearchResults> {
        let crate_name = arguments.crate_name.trim().to_owned();
        validate_crate_name(&crate_name)?;
        let query = arguments.query.trim().to_owned();
        validate_search_query(&query)?;
        let limit = arguments.limit.unwrap_or(20);
        if !(1..=100).contains(&limit) {
            bail!("limit must be between 1 and 100");
        }
        let version = match arguments.version.as_deref() {
            Some(version) => {
                validate_version(version)?;
                version.to_owned()
            }
            None => resolve_locked_version(&self.project_root, &crate_name)?,
        };

        let cache_path = self.cache_path(&crate_name, &version);
        let zstd_cache_path = cache_path.with_file_name("default.json.zst");
        let (rustdoc, source, cache_warning) = self
            .load_crate_index(
                &crate_name,
                &version,
                &cache_path,
                &zstd_cache_path,
                arguments.refresh,
            )
            .await?;
        let root = rustdoc.root_item()?;
        let crate_root = root
            .name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&crate_name);
        let docs_url = crate_docs_url(&crate_name, &version, crate_root)?;
        let results = rustdoc.search_items(&query, limit)?;

        Ok(CrateSearchResults {
            crate_name,
            version,
            query,
            results,
            docs_url,
            source,
            cache_warning,
        })
    }

    async fn load_crate_index(
        &self,
        crate_name: &str,
        version: &str,
        cache_path: &Path,
        zstd_cache_path: &Path,
        refresh: bool,
    ) -> anyhow::Result<(RustdocCrate, &'static str, Option<String>)> {
        let mut cache_problem = None;
        if !refresh {
            match read_cache_entry(zstd_cache_path, version, MAX_UNCOMPRESSED_BYTES) {
                Ok(Some((rustdoc, _))) => {
                    let _ = touch_cache_file(zstd_cache_path);
                    let _ = self.enforce_cache_limit(&[cache_path, zstd_cache_path]);
                    return Ok((rustdoc, "cache", None));
                }
                Ok(None) => {}
                Err(problem) => cache_problem = Some(problem),
            }

            match read_cache_entry(cache_path, version, MAX_COMPRESSED_BYTES) {
                Ok(Some((rustdoc, compressed))) => {
                    let _ = touch_cache_file(cache_path);
                    let mut cache_warnings = Vec::new();
                    if !zstd_cache_path.is_file() {
                        let sidecar_result = decode_json_payload(&compressed)
                            .and_then(|json| compress_zstd_json(&json))
                            .and_then(|zstd| {
                                self.write_cache(zstd_cache_path, &zstd, MAX_UNCOMPRESSED_BYTES)
                            });
                        if let Err(error) = sidecar_result {
                            cache_warnings.push(format!(
                                "could not create the zstd documentation cache: {error:#}"
                            ));
                        }
                    }
                    if let Err(error) = self.enforce_cache_limit(&[cache_path, zstd_cache_path]) {
                        cache_warnings.push(error.to_string());
                    }
                    return Ok((
                        rustdoc,
                        "cache",
                        (!cache_warnings.is_empty()).then(|| cache_warnings.join("; ")),
                    ));
                }
                Ok(None) => {}
                Err(problem) => cache_problem = Some(problem),
            }
        }

        let url = docs_json_url(crate_name, version)?;
        let compressed = self.download(&url).await.map_err(|error| {
            let context = format!("no usable cached docs for {crate_name} {version}");
            match cache_problem.as_deref() {
                Some(problem) => anyhow!("{context}: {error:#}; {problem}"),
                None => error.context(context),
            }
        })?;
        let json = decode_json_payload(&compressed)?;
        let rustdoc = parse_json_index(&json, version)?;
        let mut cache_warnings = Vec::new();
        match normalize_compressed_json(&compressed) {
            Ok(gzip) => {
                if let Err(error) = self.write_cache(cache_path, &gzip, MAX_COMPRESSED_BYTES) {
                    cache_warnings.push(format!("could not cache docs.rs gzip data: {error:#}"));
                }
            }
            Err(error) => cache_warnings.push(format!("could not prepare gzip cache: {error:#}")),
        }
        match compress_zstd_json(&json) {
            Ok(zstd) => {
                if let Err(error) = self.write_cache(zstd_cache_path, &zstd, MAX_UNCOMPRESSED_BYTES)
                {
                    cache_warnings.push(format!("could not cache zstd Rustdoc JSON: {error:#}"));
                }
            }
            Err(error) => cache_warnings.push(format!("could not prepare zstd cache: {error:#}")),
        }
        if let Err(error) = self.enforce_cache_limit(&[cache_path, zstd_cache_path]) {
            cache_warnings.push(error.to_string());
        }
        let cache_warning = (!cache_warnings.is_empty()).then(|| cache_warnings.join("; "));
        Ok((rustdoc, "docs.rs", cache_warning))
    }

    async fn download(&self, url: &Url) -> anyhow::Result<Vec<u8>> {
        let mut response = self.http.get(url.clone()).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            bail!("docs.rs has no Rustdoc JSON for this crate version (HTTP 404)");
        }
        if !response.status().is_success() {
            bail!("docs.rs returned HTTP {}", response.status());
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_COMPRESSED_BYTES)
        {
            bail!("docs.rs Rustdoc JSON exceeds the {MAX_COMPRESSED_BYTES}-byte download limit");
        }

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() as u64 + chunk.len() as u64 > MAX_COMPRESSED_BYTES {
                bail!(
                    "docs.rs Rustdoc JSON exceeds the {MAX_COMPRESSED_BYTES}-byte download limit"
                );
            }
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            bail!("docs.rs returned an empty Rustdoc JSON response");
        }
        Ok(body)
    }

    fn cache_path(&self, crate_name: &str, version: &str) -> PathBuf {
        self.cache_root
            .join(crate_name)
            .join(version)
            .join("default.json.gz")
    }

    fn write_cache(
        &self,
        cache_path: &Path,
        compressed: &[u8],
        max_entry_bytes: u64,
    ) -> anyhow::Result<()> {
        if compressed.len() as u64 > max_entry_bytes {
            bail!("documentation exceeds the {max_entry_bytes}-byte cache entry limit");
        }
        let parent = cache_path
            .parent()
            .ok_or_else(|| anyhow!("invalid docs.rs cache path"))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("creating docs.rs cache directory {}", parent.display()))?;

        let file_name = cache_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("default.json.gz");
        let temporary = parent.join(format!("{file_name}.{}.tmp", Uuid::new_v4()));
        let write_result = (|| -> anyhow::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(compressed)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, cache_path).with_context(|| {
                format!("moving docs.rs cache file into {}", cache_path.display())
            })?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }

    fn enforce_cache_limit(&self, protected: &[&Path]) -> anyhow::Result<()> {
        let mut files = Vec::new();
        collect_cache_files(&self.cache_root, &mut files)?;
        let mut total = files.iter().map(|file| file.size).sum::<u64>();
        if total <= MAX_CACHE_BYTES {
            return Ok(());
        }

        files.sort_by_key(|file| file.modified);
        for file in files {
            if protected.iter().any(|path| *path == file.path.as_path()) {
                continue;
            }
            fs::remove_file(&file.path).with_context(|| {
                format!("evicting old docs.rs cache file {}", file.path.display())
            })?;
            total = total.saturating_sub(file.size);
            if total <= MAX_CACHE_BYTES {
                return Ok(());
            }
        }
        if total > MAX_CACHE_BYTES {
            bail!("docs.rs cache exceeds its {MAX_CACHE_BYTES}-byte limit");
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct CrateDocumentation {
    crate_name: String,
    version: String,
    item_path: String,
    item_kind: String,
    documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trait_details: Option<TraitDetails>,
    docs_url: String,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_warning: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CrateSearchResults {
    crate_name: String,
    version: String,
    query: String,
    results: Vec<CrateSearchResult>,
    docs_url: String,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_warning: Option<String>,
}

#[derive(Debug, Serialize)]
struct CrateSearchResult {
    name: String,
    item_path: String,
    item_kind: String,
    matched_in: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TraitDetails {
    /// The trait declaration, including generic parameters and supertraits.
    signature: String,
    /// A compact Rust-like declaration containing the trait's method signatures.
    definition: String,
    /// Every documented method declared directly by the trait.
    methods: Vec<TraitMethodDocumentation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TraitMethodDocumentation {
    name: String,
    required: bool,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Lockfile {
    #[serde(default)]
    package: Vec<LockfilePackage>,
}

#[derive(Debug, Deserialize)]
struct LockfilePackage {
    name: String,
    version: String,
    source: Option<String>,
}

fn resolve_locked_version(project_root: &Path, crate_name: &str) -> anyhow::Result<String> {
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

fn is_crates_io_registry(source: &str) -> bool {
    source.starts_with("registry+")
        && (source.ends_with("crates.io-index") || source.contains("index.crates.io"))
}

fn validate_crate_name(crate_name: &str) -> anyhow::Result<()> {
    if crate_name.is_empty()
        || !crate_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("crate_name must contain only ASCII letters, digits, hyphens, or underscores");
    }
    Ok(())
}

fn validate_search_query(query: &str) -> anyhow::Result<()> {
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

fn short_summary(documentation: &str) -> Option<String> {
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

fn validate_version(version: &str) -> anyhow::Result<()> {
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

fn docs_json_url(crate_name: &str, version: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(DOCS_RS_JSON_URL)?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("could not construct docs.rs URL"))?
        .push(crate_name)
        .push(version)
        .push("json.gz");
    Ok(url)
}

fn crate_docs_url(crate_name: &str, version: &str, crate_root: &str) -> anyhow::Result<String> {
    let mut url = Url::parse("https://docs.rs/")?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("could not construct docs.rs URL"))?
        .pop_if_empty()
        .push(crate_name)
        .push(version)
        .push(crate_root);
    Ok(url.to_string())
}

#[derive(Debug, Deserialize)]
struct RustdocCrate {
    #[serde(deserialize_with = "deserialize_id")]
    root: String,
    format_version: u32,
    #[serde(default)]
    crate_version: Option<String>,
    index: HashMap<String, RustdocItem>,
}

#[derive(Debug, Deserialize)]
struct RustdocItem {
    crate_id: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    docs: Option<String>,
    inner: serde_json::Value,
}

struct SelectedItem {
    path: String,
    kind: String,
    docs: Option<String>,
    trait_details: Option<TraitDetails>,
}

impl RustdocCrate {
    fn select_item(&self, requested_path: Option<&str>) -> anyhow::Result<SelectedItem> {
        let root = self.root_item()?;
        let root_name = root
            .name
            .as_deref()
            .ok_or_else(|| anyhow!("Rustdoc JSON root item has no name"))?;
        let Some(requested_path) = requested_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            return Ok(SelectedItem {
                path: root_name.to_owned(),
                kind: "module".to_owned(),
                docs: root.docs.clone(),
                trait_details: None,
            });
        };

        let mut requested = requested_path
            .trim_start_matches("::")
            .split("::")
            .collect::<Vec<_>>();
        if requested
            .first()
            .is_some_and(|segment| *segment == root_name)
        {
            requested.remove(0);
        }
        if requested.iter().any(|segment| segment.is_empty()) {
            bail!("item_path contains an empty path segment");
        }
        if requested.is_empty() {
            return Ok(SelectedItem {
                path: root_name.to_owned(),
                kind: "module".to_owned(),
                docs: root.docs.clone(),
                trait_details: None,
            });
        }

        let mut matches: Vec<(String, String, Option<String>, Option<TraitDetails>)> = Vec::new();
        let mut path = vec![root_name.to_owned()];
        let mut ancestry = std::collections::HashSet::new();
        self.collect_matches(
            &self.root,
            &mut path,
            &requested,
            root.crate_id,
            &mut ancestry,
            &mut matches,
        );
        matches.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        matches.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1 && left.2 == right.2);

        match matches.as_slice() {
            [(path, kind, docs, trait_details)] => Ok(SelectedItem {
                path: path.clone(),
                kind: kind.clone(),
                docs: docs.clone(),
                trait_details: trait_details.clone(),
            }),
            [] => bail!(
                "item '{}' was not found in the public docs for this crate version",
                requested.join("::")
            ),
            many => bail!(
                "item '{}' is ambiguous in this crate version: {}",
                requested.join("::"),
                many.iter()
                    .map(|(path, kind, _, _)| format!("{path} ({kind})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn search_items(&self, query: &str, limit: usize) -> anyhow::Result<Vec<CrateSearchResult>> {
        let root = self.root_item()?;
        let root_name = root
            .name
            .as_deref()
            .ok_or_else(|| anyhow!("Rustdoc JSON root item has no name"))?;
        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();
        let mut path = vec![root_name.to_owned()];
        let mut ancestry = std::collections::HashSet::new();
        self.collect_search_matches(
            &self.root,
            &mut path,
            root.crate_id,
            &query_lower,
            &mut ancestry,
            &mut matches,
        );
        matches.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.item_path.cmp(&right.1.item_path))
                .then(left.1.item_kind.cmp(&right.1.item_kind))
        });
        matches.dedup_by(|left, right| {
            left.1.item_path == right.1.item_path && left.1.item_kind == right.1.item_kind
        });

        Ok(matches
            .into_iter()
            .take(limit)
            .map(|(_, result)| result)
            .collect())
    }

    fn collect_search_matches(
        &self,
        item_id: &str,
        path: &mut Vec<String>,
        local_crate_id: u32,
        query_lower: &str,
        ancestry: &mut std::collections::HashSet<String>,
        matches: &mut Vec<(u8, CrateSearchResult)>,
    ) {
        if !ancestry.insert(item_id.to_owned()) {
            return;
        }

        let Some(item) = self.index.get(item_id) else {
            ancestry.remove(item_id);
            return;
        };
        if item.crate_id == local_crate_id {
            let (effective, kind) = self.effective_item(item);
            let name = item
                .name
                .as_deref()
                .or_else(|| path.last().map(String::as_str))
                .unwrap_or_default();
            let item_path = path.join("::");
            let name_lower = name.to_lowercase();
            let path_lower = item_path.to_lowercase();
            let docs_lower = effective.docs.as_deref().unwrap_or_default().to_lowercase();
            let name_match = name_lower.contains(query_lower);
            let path_match = path_lower.contains(query_lower);
            let docs_match = docs_lower.contains(query_lower);

            if name_match || path_match || docs_match {
                let mut matched_in = Vec::new();
                if name_match {
                    matched_in.push("name".to_owned());
                }
                if path_match {
                    matched_in.push("path".to_owned());
                }
                if docs_match {
                    matched_in.push("documentation".to_owned());
                }
                let rank = if path_lower == query_lower {
                    0
                } else if name_lower == query_lower {
                    1
                } else if name_lower.starts_with(query_lower) {
                    2
                } else if path_match {
                    3
                } else {
                    4
                };
                matches.push((
                    rank,
                    CrateSearchResult {
                        name: name.to_owned(),
                        item_path,
                        item_kind: kind,
                        matched_in,
                        summary: effective.docs.as_deref().and_then(short_summary),
                    },
                ));
            }
        }

        for child_id in self.child_item_ids(item) {
            let Some(child) = self.index.get(&child_id) else {
                continue;
            };
            if child.crate_id != local_crate_id {
                continue;
            }
            let name = child.name.as_deref().filter(|name| !name.is_empty());
            if let Some(name) = name {
                path.push(name.to_owned());
            }
            self.collect_search_matches(
                &child_id,
                path,
                local_crate_id,
                query_lower,
                ancestry,
                matches,
            );
            if name.is_some() {
                path.pop();
            }
        }
        ancestry.remove(item_id);
    }

    fn collect_matches(
        &self,
        item_id: &str,
        path: &mut Vec<String>,
        requested: &[&str],
        local_crate_id: u32,
        ancestry: &mut std::collections::HashSet<String>,
        matches: &mut Vec<(String, String, Option<String>, Option<TraitDetails>)>,
    ) {
        if !ancestry.insert(item_id.to_owned()) {
            return;
        }

        let Some(item) = self.index.get(item_id) else {
            ancestry.remove(item_id);
            return;
        };
        let relative_path = &path[1..];
        if item.crate_id == local_crate_id
            && relative_path
                .iter()
                .map(String::as_str)
                .eq(requested.iter().copied())
        {
            let (effective, kind) = self.effective_item(item);
            let trait_details = (kind == "trait").then(|| self.trait_details(effective));
            matches.push((path.join("::"), kind, effective.docs.clone(), trait_details));
        }

        for child_id in self.child_item_ids(item) {
            let Some(child) = self.index.get(&child_id) else {
                continue;
            };
            if child.crate_id != local_crate_id {
                continue;
            }
            let Some(name) = child.name.as_deref().filter(|name| !name.is_empty()) else {
                continue;
            };
            path.push(name.to_owned());
            self.collect_matches(
                &child_id,
                path,
                requested,
                local_crate_id,
                ancestry,
                matches,
            );
            path.pop();
        }
        ancestry.remove(item_id);
    }

    fn child_item_ids(&self, item: &RustdocItem) -> Vec<String> {
        let Some(inner) = item.inner.as_object() else {
            return Vec::new();
        };
        if let Some(module) = inner.get("module") {
            return ids_in(module.get("items"));
        }
        if let Some(trait_) = inner.get("trait") {
            return ids_in(trait_.get("items"));
        }
        if let Some(enumeration) = inner.get("enum") {
            let mut ids = ids_in(enumeration.get("variants"));
            ids.extend(impl_item_ids(self, enumeration.get("impls")));
            return ids;
        }
        if let Some(structure) = inner.get("struct") {
            let mut ids = Vec::new();
            if let Some(kind) = structure.get("kind") {
                if let Some(plain) = kind.get("plain") {
                    ids.extend(ids_in(plain.get("fields")));
                }
                if let Some(tuple) = kind.get("tuple") {
                    ids.extend(ids_in(Some(tuple)));
                }
            }
            ids.extend(impl_item_ids(self, structure.get("impls")));
            return ids;
        }
        if let Some(union) = inner.get("union") {
            let mut ids = ids_in(union.get("fields"));
            ids.extend(impl_item_ids(self, union.get("impls")));
            return ids;
        }
        if let Some(variant) = inner.get("variant") {
            return ids_in(variant.get("fields"));
        }
        if let Some(implementation) = inner.get("impl") {
            return ids_in(implementation.get("items"));
        }
        if let Some(use_item) = inner.get("use") {
            return id_in(use_item.get("id")).into_iter().collect();
        }
        Vec::new()
    }

    fn effective_item<'a>(&'a self, item: &'a RustdocItem) -> (&'a RustdocItem, String) {
        let kind = item
            .inner
            .as_object()
            .and_then(|inner| inner.keys().next())
            .cloned()
            .unwrap_or_else(|| "item".to_owned());
        let target = item
            .inner
            .get("use")
            .and_then(|use_item| id_in(use_item.get("id")))
            .and_then(|id| self.index.get(&id));
        match target {
            Some(target) => {
                let target_kind = target
                    .inner
                    .as_object()
                    .and_then(|inner| inner.keys().next())
                    .cloned()
                    .unwrap_or(kind);
                (target, target_kind)
            }
            None => (item, kind),
        }
    }

    fn trait_details(&self, item: &RustdocItem) -> TraitDetails {
        let trait_data = item.inner.get("trait").unwrap_or(&serde_json::Value::Null);
        let name = item.name.as_deref().unwrap_or("Trait");
        let signature = format_trait_signature(name, trait_data);
        let methods = trait_data
            .get("items")
            .map(|items| ids_in(Some(items)))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| self.index.get(&id))
            .filter_map(|method| {
                let name = method.name.as_deref()?.to_owned();
                let function = method.inner.get("function")?;
                let required = !function
                    .get("has_body")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                Some(TraitMethodDocumentation {
                    name: name.clone(),
                    required,
                    signature: format_method_signature(&name, function),
                    documentation: method.docs.clone(),
                })
            })
            .collect::<Vec<_>>();
        let definition = format_trait_definition(&signature, &methods);

        TraitDetails {
            signature,
            definition,
            methods,
        }
    }

    fn root_item(&self) -> anyhow::Result<&RustdocItem> {
        self.index
            .get(&self.root)
            .ok_or_else(|| anyhow!("Rustdoc JSON root item is missing from its index"))
    }
}

fn format_trait_signature(name: &str, trait_data: &serde_json::Value) -> String {
    let unsafe_modifier = trait_data
        .get("is_unsafe")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        .then_some("unsafe ")
        .unwrap_or_default();
    let auto_modifier = trait_data
        .get("is_auto")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        .then_some("auto ")
        .unwrap_or_default();
    let generics = trait_data
        .get("generics")
        .and_then(|generics| generics.get("params"))
        .and_then(serde_json::Value::as_array)
        .map(|params| format_generic_params(params, true))
        .unwrap_or_default();
    let bounds = trait_data
        .get("bounds")
        .and_then(serde_json::Value::as_array)
        .map(|bounds| {
            bounds
                .iter()
                .map(format_generic_bound)
                .filter(|bound| !bound.is_empty())
                .collect::<Vec<_>>()
                .join(" + ")
        })
        .unwrap_or_default();
    let supertraits = if bounds.is_empty() {
        String::new()
    } else {
        format!(": {bounds}")
    };
    let where_clauses = trait_data
        .get("generics")
        .and_then(|generics| generics.get("where_predicates"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(format_where_predicate)
        .collect::<Vec<_>>();
    let where_clause = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("\nwhere\n    {}", where_clauses.join(",\n    "))
    };

    format!("pub {unsafe_modifier}{auto_modifier}trait {name}{generics}{supertraits}{where_clause}")
}

fn format_trait_definition(signature: &str, methods: &[TraitMethodDocumentation]) -> String {
    let mut lines = vec![format!("{signature} {{")];
    for method in methods {
        let method_kind = if method.required {
            "Required"
        } else {
            "Provided"
        };
        lines.push(format!("    // {method_kind} method"));
        let mut declaration = method.signature.clone();
        if method.required {
            declaration.push(';');
        } else if declaration.contains('\n') {
            declaration.push_str("\n{");
            declaration.push_str("\n    /* provided method body */\n}");
        } else {
            declaration.push_str(" { /* provided method body */ }");
        }
        lines.extend(declaration.lines().map(|line| format!("    {line}")));
    }
    lines.push("}".to_owned());
    lines.join("\n")
}

fn format_method_signature(name: &str, function: &serde_json::Value) -> String {
    let signature = function.get("sig").unwrap_or(&serde_json::Value::Null);
    let generics = function.get("generics").unwrap_or(&serde_json::Value::Null);
    let generic_parameters = generics
        .get("params")
        .and_then(serde_json::Value::as_array)
        .map(|params| format_generic_params(params, false))
        .unwrap_or_default();
    let header = function.get("header").unwrap_or(&serde_json::Value::Null);
    let mut modifiers = Vec::new();
    for (field, keyword) in [
        ("is_const", "const"),
        ("is_async", "async"),
        ("is_unsafe", "unsafe"),
    ] {
        if header
            .get(field)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            modifiers.push(keyword.to_owned());
        }
    }
    if let Some(abi) = header
        .get("abi")
        .and_then(serde_json::Value::as_str)
        .filter(|abi| !abi.is_empty() && *abi != "Rust")
    {
        modifiers.push(format!("extern \"{abi}\""));
    }
    let modifiers = if modifiers.is_empty() {
        String::new()
    } else {
        format!("{} ", modifiers.join(" "))
    };
    let inputs = signature
        .get("inputs")
        .and_then(serde_json::Value::as_array)
        .map(|inputs| {
            inputs
                .iter()
                .map(format_function_input)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let mut rendered = format!("{modifiers}fn {name}{generic_parameters}({inputs})");
    if let Some(output) = signature.get("output").filter(|output| !output.is_null()) {
        rendered.push_str(" -> ");
        rendered.push_str(&format_type(output));
    }
    let where_clauses = format_where_clauses(generics);
    if !where_clauses.is_empty() {
        rendered.push_str("\nwhere\n    ");
        rendered.push_str(&where_clauses.join(",\n    "));
    }
    rendered
}

fn format_function_input(input: &serde_json::Value) -> String {
    let Some(parts) = input.as_array() else {
        return format_type(input);
    };
    let Some(name) = parts.first().and_then(serde_json::Value::as_str) else {
        return parts
            .get(1)
            .map(format_type)
            .unwrap_or_else(|| "_".to_owned());
    };
    let Some(ty) = parts.get(1) else {
        return name.to_owned();
    };
    if name == "self" {
        if let Some(reference) = ty.get("borrowed_ref") {
            let lifetime = reference
                .get("lifetime")
                .and_then(serde_json::Value::as_str)
                .filter(|lifetime| !lifetime.is_empty())
                .map(|lifetime| format!("{lifetime} "))
                .unwrap_or_default();
            let mutable = reference
                .get("is_mutable")
                .or_else(|| reference.get("mutable"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            return format!("&{lifetime}{}self", if mutable { "mut " } else { "" });
        }
        if type_is_self(ty) {
            return "self".to_owned();
        }
        return format!("self: {}", format_type(ty));
    }
    format!("{name}: {}", format_type(ty))
}

fn type_is_self(ty: &serde_json::Value) -> bool {
    ty.get("generic").and_then(serde_json::Value::as_str) == Some("Self")
}

fn format_generic_params(params: &[serde_json::Value], include_bounds: bool) -> String {
    let rendered = params
        .iter()
        .map(|param| format_generic_param(param, include_bounds))
        .filter(|param| !param.is_empty())
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        String::new()
    } else {
        format!("<{}>", rendered.join(", "))
    }
}

fn format_generic_param(param: &serde_json::Value, include_bounds: bool) -> String {
    let Some(name) = param.get("name").and_then(serde_json::Value::as_str) else {
        return String::new();
    };
    let kind = param.get("kind").unwrap_or(&serde_json::Value::Null);
    if let Some(lifetime) = kind.get("lifetime") {
        let bounds = lifetime
            .get("outlives")
            .and_then(serde_json::Value::as_array)
            .map(|bounds| {
                bounds
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" + ")
            })
            .unwrap_or_default();
        return if include_bounds && !bounds.is_empty() {
            format!("{name}: {bounds}")
        } else {
            name.to_owned()
        };
    }
    if let Some(type_param) = kind.get("type") {
        let bounds = type_param
            .get("bounds")
            .and_then(serde_json::Value::as_array)
            .map(|bounds| {
                bounds
                    .iter()
                    .map(format_generic_bound)
                    .filter(|bound| !bound.is_empty())
                    .collect::<Vec<_>>()
                    .join(" + ")
            })
            .unwrap_or_default();
        let default = type_param
            .get("default")
            .filter(|default| !default.is_null())
            .map(format_type)
            .unwrap_or_default();
        if include_bounds && !bounds.is_empty() {
            return if default.is_empty() {
                format!("{name}: {bounds}")
            } else {
                format!("{name}: {bounds} = {default}")
            };
        }
        return if include_bounds && !default.is_empty() {
            format!("{name} = {default}")
        } else {
            name.to_owned()
        };
    }
    if let Some(constant) = kind.get("const") {
        let ty = constant
            .get("type")
            .map(format_type)
            .unwrap_or_else(|| "_".to_owned());
        let default = constant
            .get("default")
            .and_then(serde_json::Value::as_str)
            .map(|default| format!(" = {default}"))
            .unwrap_or_default();
        return format!("const {name}: {ty}{default}");
    }
    name.to_owned()
}

fn format_where_clauses(generics: &serde_json::Value) -> Vec<String> {
    let mut clauses = Vec::new();
    if let Some(params) = generics.get("params").and_then(serde_json::Value::as_array) {
        for param in params {
            let Some(name) = param.get("name").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let kind = param.get("kind").unwrap_or(&serde_json::Value::Null);
            let bounds = if let Some(type_param) = kind.get("type") {
                type_param
                    .get("bounds")
                    .and_then(serde_json::Value::as_array)
                    .map(|bounds| {
                        bounds
                            .iter()
                            .map(format_generic_bound)
                            .filter(|bound| !bound.is_empty())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else if let Some(lifetime) = kind.get("lifetime") {
                lifetime
                    .get("outlives")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            } else {
                Vec::new()
            };
            if !bounds.is_empty() {
                clauses.push(format!("{name}: {}", bounds.join(" + ")));
            }
        }
    }
    if let Some(predicates) = generics
        .get("where_predicates")
        .and_then(serde_json::Value::as_array)
    {
        clauses.extend(predicates.iter().filter_map(format_where_predicate));
    }
    let mut unique = Vec::new();
    for clause in clauses {
        if !unique.contains(&clause) {
            unique.push(clause);
        }
    }
    unique
}

fn format_where_predicate(predicate: &serde_json::Value) -> Option<String> {
    let (kind, predicate) = predicate.as_object()?.iter().next()?;
    if kind == "bound_predicate" {
        let target = predicate.get("type").map(format_type)?;
        let bounds = predicate
            .get("bounds")
            .and_then(serde_json::Value::as_array)?
            .iter()
            .map(format_generic_bound)
            .filter(|bound| !bound.is_empty())
            .collect::<Vec<_>>();
        if bounds.is_empty() {
            return None;
        }
        let generic_params = predicate
            .get("generic_params")
            .and_then(serde_json::Value::as_array)
            .map(|params| format_generic_params(params, true))
            .unwrap_or_default();
        let for_prefix = if generic_params.is_empty() {
            String::new()
        } else {
            format!("for{generic_params} ")
        };
        return Some(format!("{for_prefix}{target}: {}", bounds.join(" + ")));
    }
    if kind == "lifetime_predicate" {
        let name = predicate
            .get("lifetime")
            .and_then(serde_json::Value::as_str)?;
        let outlives = predicate
            .get("outlives")
            .and_then(serde_json::Value::as_array)?
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        return (!outlives.is_empty()).then(|| format!("{name}: {}", outlives.join(" + ")));
    }
    if kind == "eq_predicate" {
        let left = predicate.get("lhs").map(format_type)?;
        let right = predicate.get("rhs").map(format_type)?;
        return Some(format!("{left} = {right}"));
    }
    None
}

fn format_generic_bound(bound: &serde_json::Value) -> String {
    let Some(bounds) = bound.as_object() else {
        return String::new();
    };
    if let Some(trait_bound) = bounds.get("trait_bound") {
        return format_poly_trait(
            trait_bound,
            matches!(
                trait_bound
                    .get("modifier")
                    .and_then(serde_json::Value::as_str),
                Some("maybe") | Some("maybe_const")
            ),
        );
    }
    if bounds.contains_key("trait") {
        return format_poly_trait(bound, false);
    }
    bounds
        .get("outlives")
        .and_then(serde_json::Value::as_str)
        .or_else(|| bounds.get("lifetime").and_then(serde_json::Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn format_generic_args(args: &serde_json::Value) -> String {
    if let Some(angle) = args.get("angle_bracketed") {
        let mut values = angle
            .get("args")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .map(format_generic_arg)
            .filter(|arg| !arg.is_empty())
            .collect::<Vec<_>>();
        values.extend(
            angle
                .get("bindings")
                .or_else(|| angle.get("constraints"))
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .map(format_associated_binding)
                .filter(|binding| !binding.is_empty()),
        );
        if values.is_empty() {
            String::new()
        } else {
            format!("<{}>", values.join(", "))
        }
    } else if let Some(parenthesized) = args.get("parenthesized") {
        let inputs = parenthesized
            .get("inputs")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .map(format_type)
            .collect::<Vec<_>>()
            .join(", ");
        let output = parenthesized
            .get("output")
            .filter(|output| !output.is_null())
            .map(|output| format!(" -> {}", format_type(output)))
            .unwrap_or_default();
        format!("({inputs}){output}")
    } else {
        String::new()
    }
}

fn format_generic_arg(arg: &serde_json::Value) -> String {
    let Some((kind, value)) = arg.as_object().and_then(|args| args.iter().next()) else {
        return String::new();
    };
    match kind.as_str() {
        "type" => format_type(value),
        "lifetime" => value.as_str().unwrap_or_default().to_owned(),
        "const" => value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string()),
        "infer" => "_".to_owned(),
        _ => String::new(),
    }
}

fn format_associated_binding(binding: &serde_json::Value) -> String {
    let Some(name) = binding.get("name").and_then(serde_json::Value::as_str) else {
        return String::new();
    };
    if let Some(value) = binding.get("type") {
        return format!("{name} = {}", format_type(value));
    }
    if let Some(binding) = binding.get("binding") {
        let bounds = binding
            .get("constraints")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .map(format_generic_bound)
            .filter(|bound| !bound.is_empty())
            .collect::<Vec<_>>();
        if !bounds.is_empty() {
            return format!("{name}: {}", bounds.join(" + "));
        }
    }
    String::new()
}

fn format_type(ty: &serde_json::Value) -> String {
    let Some((kind, value)) = ty.as_object().and_then(|types| types.iter().next()) else {
        return "_".to_owned();
    };
    match kind.as_str() {
        "resolved_path" => {
            let path = value
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("_");
            let args = value
                .get("args")
                .map(format_generic_args)
                .unwrap_or_default();
            format!("{path}{args}")
        }
        "generic" | "primitive" => value.as_str().unwrap_or("_").to_owned(),
        "borrowed_ref" => {
            let lifetime = value
                .get("lifetime")
                .and_then(serde_json::Value::as_str)
                .filter(|lifetime| !lifetime.is_empty())
                .map(|lifetime| format!("{lifetime} "))
                .unwrap_or_default();
            let mutable = value
                .get("is_mutable")
                .or_else(|| value.get("mutable"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let pointee = value
                .get("type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned());
            format!("&{lifetime}{}{pointee}", if mutable { "mut " } else { "" })
        }
        "raw_pointer" => {
            let mutable = value
                .get("is_mutable")
                .or_else(|| value.get("mutable"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let pointee = value
                .get("type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned());
            format!("*{} {pointee}", if mutable { "mut" } else { "const" })
        }
        "tuple" => format!(
            "({})",
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(format_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "slice" => format!(
            "[{}]",
            value
                .get("type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned())
        ),
        "array" => format!(
            "[{}; {}]",
            value
                .get("type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned()),
            value
                .get("len")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("_")
        ),
        "never" => "!".to_owned(),
        "infer" => "_".to_owned(),
        "qualified_path" => {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("_");
            let self_type = value
                .get("self_type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned());
            let trait_type = value
                .get("trait")
                .or_else(|| value.get("trait_"))
                .filter(|trait_| !trait_.is_null())
                .map(format_path)
                .filter(|path| !path.is_empty() && path != "_")
                .map(|path| format!(" as {path}"))
                .unwrap_or_default();
            let args = value
                .get("args")
                .map(format_generic_args)
                .unwrap_or_default();
            if trait_type.is_empty() {
                format!("{self_type}::{name}{args}")
            } else {
                format!("<{self_type}{trait_type}>::{name}{args}")
            }
        }
        "dyn_trait" => {
            let traits = value
                .get("traits")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .map(format_generic_bound)
                .filter(|bound| !bound.is_empty())
                .collect::<Vec<_>>()
                .join(" + ");
            format!("dyn {traits}")
        }
        "impl_trait" => format!(
            "impl {}",
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(format_generic_bound)
                .filter(|bound| !bound.is_empty())
                .collect::<Vec<_>>()
                .join(" + ")
        ),
        _ => "_".to_owned(),
    }
}

fn format_poly_trait(trait_bound: &serde_json::Value, maybe: bool) -> String {
    let path = trait_bound.get("trait").unwrap_or(&serde_json::Value::Null);
    let generic_params = trait_bound
        .get("generic_params")
        .and_then(serde_json::Value::as_array)
        .map(|params| format_generic_params(params, true))
        .unwrap_or_default();
    let for_prefix = if generic_params.is_empty() {
        String::new()
    } else {
        format!("for{generic_params} ")
    };
    format!(
        "{for_prefix}{}{path}",
        if maybe { "?" } else { "" },
        path = format_path(path)
    )
}

fn format_path(path: &serde_json::Value) -> String {
    let name = path
        .get("path")
        .and_then(serde_json::Value::as_str)
        .or_else(|| path.as_str())
        .unwrap_or("_");
    let args = path
        .get("args")
        .filter(|args| !args.is_null())
        .map(format_generic_args)
        .unwrap_or_default();
    format!("{name}{args}")
}

fn parse_compressed_index(compressed: &[u8], version: &str) -> anyhow::Result<RustdocCrate> {
    let json = decode_json_payload(compressed)?;
    parse_json_index(&json, version)
}

fn parse_json_index(json: &[u8], version: &str) -> anyhow::Result<RustdocCrate> {
    let rustdoc: RustdocCrate =
        serde_json::from_slice(json).context("parsing docs.rs Rustdoc JSON")?;
    if rustdoc.format_version == 0 {
        bail!("docs.rs returned an invalid Rustdoc JSON format version");
    }
    rustdoc.root_item()?;
    if rustdoc
        .crate_version
        .as_deref()
        .is_some_and(|actual| actual != version)
    {
        bail!("docs.rs returned Rustdoc JSON for a different crate version");
    }
    if rustdoc.index.is_empty() {
        bail!("docs.rs returned an empty Rustdoc JSON crate index");
    }
    Ok(rustdoc)
}

fn ids_in(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(id_value)
        .collect()
}

fn id_in(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(id_value)
}

fn id_value(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|id| id.to_string()))
        .or_else(|| value.get("id").and_then(id_value))
}

fn deserialize_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    id_value(&value).ok_or_else(|| {
        <D::Error as serde::de::Error>::custom("expected a string or unsigned integer item ID")
    })
}

fn impl_item_ids(rustdoc: &RustdocCrate, impl_ids: Option<&serde_json::Value>) -> Vec<String> {
    ids_in(impl_ids)
        .into_iter()
        .flat_map(|id| {
            rustdoc
                .index
                .get(&id)
                .and_then(|item| item.inner.get("impl"))
                .map(|implementation| ids_in(implementation.get("items")))
                .unwrap_or_default()
        })
        .collect()
}

fn decode_json_payload(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        if bytes.len() as u64 > MAX_COMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_COMPRESSED_BYTES}-byte gzip size limit");
        }
        let decoder = GzDecoder::new(bytes);
        let mut output = Vec::new();
        decoder
            .take(MAX_UNCOMPRESSED_BYTES + 1)
            .read_to_end(&mut output)
            .context("decompressing docs.rs Rustdoc JSON")?;
        if output.len() as u64 > MAX_UNCOMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte decompressed size limit");
        }
        Ok(output)
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        if bytes.len() as u64 > MAX_UNCOMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte zstd size limit");
        }
        let decoder = zstd::stream::read::Decoder::new(bytes)
            .context("decompressing cached zstd Rustdoc JSON")?;
        let mut output = Vec::new();
        decoder
            .take(MAX_UNCOMPRESSED_BYTES + 1)
            .read_to_end(&mut output)
            .context("decompressing cached zstd Rustdoc JSON")?;
        if output.len() as u64 > MAX_UNCOMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte decompressed size limit");
        }
        Ok(output)
    } else if bytes
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .next()
        == Some(b'{')
    {
        if bytes.len() as u64 > MAX_UNCOMPRESSED_BYTES {
            bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte decompressed size limit");
        }
        Ok(bytes.to_vec())
    } else {
        bail!("docs.rs response is neither gzip data nor Rustdoc JSON");
    }
}

fn normalize_compressed_json(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes.to_vec());
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    let compressed = encoder.finish()?;
    if compressed.len() as u64 > MAX_COMPRESSED_BYTES {
        bail!("documentation exceeds the {MAX_COMPRESSED_BYTES}-byte cache entry limit");
    }
    Ok(compressed)
}

fn compress_zstd_json(json: &[u8]) -> anyhow::Result<Vec<u8>> {
    if json.len() as u64 > MAX_UNCOMPRESSED_BYTES {
        bail!("Rustdoc JSON exceeds the {MAX_UNCOMPRESSED_BYTES}-byte size limit");
    }
    zstd::bulk::compress(json, 3).context("compressing Rustdoc JSON with zstd")
}

fn read_cache_entry(
    path: &Path,
    version: &str,
    max_entry_bytes: u64,
) -> Result<Option<(RustdocCrate, Vec<u8>)>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not read cached docs {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.len() > max_entry_bytes {
        let _ = fs::remove_file(path);
        return Err(format!(
            "cached docs {} exceeded the {max_entry_bytes}-byte limit and were removed",
            path.display()
        ));
    }

    let bytes = fs::read(path)
        .map_err(|error| format!("could not read cached docs {}: {error}", path.display()))?;
    match parse_compressed_index(&bytes, version) {
        Ok(rustdoc) => Ok(Some((rustdoc, bytes))),
        Err(error) => {
            let _ = fs::remove_file(path);
            Err(format!(
                "cached docs {} were invalid and removed: {error:#}",
                path.display()
            ))
        }
    }
}

struct CacheFile {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

fn collect_cache_files(root: &Path, files: &mut Vec<CacheFile>) -> anyhow::Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", root.display())),
    };
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_cache_files(&entry.path(), files)?;
        } else if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".json.gz") || name.ends_with(".json.zst"))
        {
            let metadata = entry.metadata()?;
            files.push(CacheFile {
                path: entry.path(),
                size: metadata.len(),
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    Ok(())
}

fn touch_cache_file(path: &Path) -> std::io::Result<()> {
    let file = File::open(path)?;
    file.set_times(FileTimes::new().set_modified(SystemTime::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCS_JSON: &str = r#"{
        "root": 0,
        "format_version": 1,
        "crate_version": "1.2.3",
        "index": {
            "0": {
                "crate_id": 0,
                "name": "demo",
                "docs": "crate docs",
                "inner": { "module": { "is_crate": true, "items": [1] } }
            },
            "1": {
                "crate_id": 0,
                "name": "Guide",
                "docs": "guide docs",
                "inner": { "module": { "is_crate": false, "items": [] } }
            }
        }
    }"#;

    fn client_with_cache() -> (tempfile::TempDir, tempfile::TempDir, DocsRsClient) {
        let project = tempfile::tempdir().unwrap();
        let project_root = project.path().join("member");
        fs::create_dir_all(&project_root).unwrap();
        fs::write(
            project.path().join("Cargo.lock"),
            "[[package]]\nname = \"demo\"\nversion = \"1.2.3\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        )
        .unwrap();

        let cache = tempfile::tempdir().unwrap();
        let mut client = DocsRsClient::new(project_root).unwrap();
        client.cache_root = cache.path().to_path_buf();
        (project, cache, client)
    }

    fn cached_arguments() -> GetCrateDocsArguments {
        GetCrateDocsArguments {
            crate_name: "demo".to_owned(),
            item_path: Some("Guide".to_owned()),
            version: None,
            refresh: false,
        }
    }

    fn get_cached_document(client: &DocsRsClient) -> CrateDocumentation {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(2),
                    client.get_document(cached_arguments()),
                )
                .await
                .expect("a cache hit should not make a network request")
            })
            .expect("cached documentation should parse")
    }

    #[test]
    fn trait_lookup_returns_definition_required_status_and_method_documentation() {
        let json = serde_json::json!({
            "root": 0,
            "format_version": 1,
            "crate_version": "1.2.3",
            "index": {
                "0": {
                    "crate_id": 0,
                    "name": "demo",
                    "docs": "crate docs",
                    "inner": { "module": { "items": [1] } }
                },
                "1": {
                    "crate_id": 0,
                    "name": "Deserialize",
                    "docs": "A data structure that can be deserialized.\n\nExpanded trait description.",
                    "inner": {
                        "trait": {
                            "is_auto": false,
                            "is_unsafe": false,
                            "is_dyn_compatible": false,
                            "items": [2],
                            "generics": {
                                "params": [{
                                    "name": "'de",
                                    "kind": { "lifetime": { "outlives": [] } }
                                }],
                                "where_predicates": []
                            },
                            "bounds": [{
                                "trait_bound": {
                                    "trait": { "path": "Sized", "id": 3, "args": null },
                                    "generic_params": [],
                                    "modifier": "none"
                                }
                            }]
                        }
                    }
                },
                "2": {
                    "crate_id": 0,
                    "name": "deserialize",
                    "docs": "Deserialize this value from the given deserializer.\n\nSee the implementation guide for details.",
                    "inner": {
                        "function": {
                            "sig": {
                                "inputs": [["deserializer", { "generic": "D" }]],
                                "output": {
                                    "resolved_path": {
                                        "path": "Result",
                                        "id": 4,
                                        "args": {
                                            "angle_bracketed": {
                                                "args": [
                                                    { "type": { "generic": "Self" } },
                                                    { "type": {
                                                        "qualified_path": {
                                                            "name": "Error",
                                                            "args": null,
                                                            "self_type": { "generic": "D" },
                                                            "trait": null
                                                        }
                                                    } }
                                                ],
                                                "bindings": []
                                            }
                                        }
                                    }
                                }
                            },
                            "generics": {
                                "params": [{
                                    "name": "D",
                                    "kind": { "type": {
                                        "bounds": [{
                                            "trait_bound": {
                                                "trait": {
                                                    "path": "Deserializer",
                                                    "id": 5,
                                                    "args": {
                                                        "angle_bracketed": {
                                                            "args": [{ "lifetime": "'de" }],
                                                            "bindings": []
                                                        }
                                                    }
                                                },
                                                "generic_params": [],
                                                "modifier": "none"
                                            }
                                        }],
                                        "default": null
                                    } }
                                }],
                                "where_predicates": []
                            },
                            "header": {
                                "is_const": false,
                                "is_unsafe": false,
                                "is_async": false,
                                "abi": "Rust"
                            },
                            "has_body": false
                        }
                    }
                }
            }
        });
        let rustdoc = parse_json_index(&serde_json::to_vec(&json).unwrap(), "1.2.3").unwrap();
        let selected = rustdoc.select_item(Some("Deserialize")).unwrap();
        let response = serde_json::to_value(CrateDocumentation {
            crate_name: "demo".to_owned(),
            version: "1.2.3".to_owned(),
            item_path: selected.path,
            item_kind: selected.kind,
            documentation: selected.docs,
            trait_details: selected.trait_details,
            docs_url: "https://docs.rs/demo/1.2.3/demo".to_owned(),
            source: "cache",
            cache_warning: None,
        })
        .unwrap();

        assert_eq!(response["item_kind"], "trait");
        assert_eq!(
            response["documentation"],
            "A data structure that can be deserialized.\n\nExpanded trait description."
        );
        assert_eq!(
            response["trait_details"]["signature"],
            "pub trait Deserialize<'de>: Sized"
        );
        assert_eq!(
            response["trait_details"]["definition"],
            concat!(
                "pub trait Deserialize<'de>: Sized {\n",
                "    // Required method\n",
                "    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\n",
                "    where\n",
                "        D: Deserializer<'de>;\n",
                "}"
            )
        );
        assert_eq!(
            response["trait_details"]["methods"][0]["name"],
            "deserialize"
        );
        assert_eq!(response["trait_details"]["methods"][0]["required"], true);
        assert_eq!(
            response["trait_details"]["methods"][0]["documentation"],
            "Deserialize this value from the given deserializer.\n\nSee the implementation guide for details."
        );
    }

    #[test]
    fn offline_lookup_reads_zstd_cache_and_resolves_version_from_parent_lockfile() {
        let (_project, _cache, client) = client_with_cache();
        let gzip_path = client.cache_path("demo", "1.2.3");
        fs::create_dir_all(gzip_path.parent().unwrap()).unwrap();
        let zstd_path = gzip_path.with_file_name("default.json.zst");
        fs::write(
            &zstd_path,
            compress_zstd_json(DOCS_JSON.as_bytes()).unwrap(),
        )
        .unwrap();

        let document = get_cached_document(&client);

        assert_eq!(document.source, "cache");
        assert_eq!(document.version, "1.2.3");
        assert_eq!(document.item_path, "demo::Guide");
        assert_eq!(document.item_kind, "module");
        assert_eq!(document.documentation.as_deref(), Some("guide docs"));
        assert_eq!(document.docs_url, "https://docs.rs/demo/1.2.3/demo");
        assert!(!gzip_path.exists());
    }

    #[test]
    fn offline_lookup_upgrades_gzip_cache_to_zstd_sidecar() {
        let (_project, _cache, client) = client_with_cache();
        let gzip_path = client.cache_path("demo", "1.2.3");
        fs::create_dir_all(gzip_path.parent().unwrap()).unwrap();
        fs::write(
            &gzip_path,
            normalize_compressed_json(DOCS_JSON.as_bytes()).unwrap(),
        )
        .unwrap();

        let document = get_cached_document(&client);
        let zstd_path = gzip_path.with_file_name("default.json.zst");

        assert_eq!(document.source, "cache");
        assert_eq!(document.documentation.as_deref(), Some("guide docs"));
        assert!(zstd_path.is_file());
        assert_eq!(
            decode_json_payload(&fs::read(zstd_path).unwrap()).unwrap(),
            DOCS_JSON.as_bytes()
        );
    }

    #[test]
    #[ignore = "contacts docs.rs over the network"]
    fn live_docs_rs_download_is_then_read_from_zstd_cache() {
        let (_project, _cache, client) = client_with_cache();
        let arguments = GetCrateDocsArguments {
            crate_name: "serde".to_owned(),
            item_path: Some("de::Deserialize".to_owned()),
            version: Some("1.0.229".to_owned()),
            refresh: false,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let downloaded = runtime
            .block_on(client.get_document(arguments.clone()))
            .unwrap();
        assert_eq!(downloaded.source, "docs.rs");
        assert!(
            downloaded
                .documentation
                .as_deref()
                .is_some_and(|docs| !docs.trim().is_empty())
        );
        let trait_details = downloaded
            .trait_details
            .as_ref()
            .expect("a trait result should contain the trait declaration");
        assert_eq!(trait_details.signature, "pub trait Deserialize<'de>: Sized");
        assert!(trait_details.definition.contains("// Required method"));
        let deserialize = trait_details
            .methods
            .iter()
            .find(|method| method.name == "deserialize")
            .expect("Deserialize should expose its required deserialize method");
        assert!(deserialize.required);
        assert_eq!(
            deserialize.signature,
            "fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\nwhere\n    D: Deserializer<'de>"
        );
        assert!(
            deserialize
                .documentation
                .as_deref()
                .is_some_and(|docs| !docs.trim().is_empty())
        );

        let gzip_path = client.cache_path("serde", "1.0.229");
        let zstd_path = gzip_path.with_file_name("default.json.zst");
        assert!(gzip_path.is_file());
        assert!(zstd_path.is_file());

        let cached = runtime.block_on(client.get_document(arguments)).unwrap();
        assert_eq!(cached.source, "cache");
        assert_eq!(cached.item_path, downloaded.item_path);
        assert_eq!(cached.documentation, downloaded.documentation);
        assert_eq!(cached.trait_details, downloaded.trait_details);
    }
}

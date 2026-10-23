//! Shared read-only Rust documentation access for the Aimer MCP server and CLI.

mod cache;
mod format;
mod index;
mod source;
mod standard;

use cache::*;
use format::*;
use source::*;

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
#[cfg(feature = "mcp")]
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const DOCS_RS_JSON_URL: &str = "https://docs.rs/crate";
const MAX_COMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct GetCrateDocsArguments {
    /// Cargo package name, for example `serde`, or a toolchain crate such as `std`, `core`, or `alloc`.
    pub(crate) crate_name: String,
    /// Rust item path relative to the crate root, for example `de::Deserialize`.
    /// If omitted, the crate-level documentation is returned.
    #[serde(default)]
    pub(crate) item_path: Option<String>,
    /// Exact crate version (defaults to Cargo.lock). For toolchain crates, a Rust toolchain
    /// such as `nightly` or `nightly-2026-10-01`; defaults to the active toolchain, then nightly.
    #[serde(default)]
    pub(crate) version: Option<String>,
    /// Refresh the cached crate index from docs.rs. Toolchain documentation is always read locally.
    #[serde(default)]
    pub(crate) refresh: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SearchCrateDocsArguments {
    /// Cargo package name, for example `serde`, or a toolchain crate such as `std`, `core`, or `alloc`.
    pub(crate) crate_name: String,
    /// Case-insensitive text to find in public item names, paths, and docs.
    pub(crate) query: String,
    /// Exact crate version (defaults to Cargo.lock). For toolchain crates, a Rust toolchain
    /// such as `nightly` or `nightly-2026-10-01`; defaults to the active toolchain, then nightly.
    #[serde(default)]
    pub(crate) version: Option<String>,
    /// Maximum number of results to return (1–100; defaults to 20).
    #[serde(default)]
    pub(crate) limit: Option<usize>,
    /// Refresh the cached crate index from docs.rs. Toolchain documentation is always read locally.
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
        if standard::is_toolchain_crate(&crate_name) {
            let (rustdoc, version, docs_url) = standard::load(
                &self.project_root,
                &crate_name,
                arguments.version.as_deref(),
            )
            .await?;
            let selected = rustdoc.select_item(arguments.item_path.as_deref())?;
            return Ok(CrateDocumentation {
                crate_name,
                version,
                item_path: selected.path,
                item_kind: selected.kind,
                documentation: selected.docs,
                trait_details: selected.trait_details,
                docs_url,
                source: "rust-toolchain",
                cache_warning: None,
            });
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
        if standard::is_toolchain_crate(&crate_name) {
            let (rustdoc, version, docs_url) = standard::load(
                &self.project_root,
                &crate_name,
                arguments.version.as_deref(),
            )
            .await?;
            let results = rustdoc.search_items(&query, limit)?;
            return Ok(CrateSearchResults {
                crate_name,
                version,
                query,
                results,
                docs_url,
                source: "rust-toolchain",
                cache_warning: None,
            });
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

#[derive(Debug, Deserialize)]
struct RustdocCrate {
    #[serde(deserialize_with = "deserialize_id")]
    root: String,
    format_version: u32,
    #[serde(default)]
    crate_version: Option<String>,
    index: HashMap<String, RustdocItem>,
    #[serde(default)]
    paths: HashMap<String, RustdocPath>,
}

#[derive(Debug, Deserialize)]
struct RustdocPath {
    path: Vec<String>,
    kind: String,
}

#[derive(Debug, Deserialize)]
struct RustdocItem {
    crate_id: u32,
    #[serde(default)]
    visibility: serde_json::Value,
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
                .map(|implementation| {
                    let trait_impl = implementation
                        .get("trait")
                        .is_some_and(|value| !value.is_null());
                    ids_in(implementation.get("items"))
                        .into_iter()
                        .filter(|id| {
                            trait_impl || rustdoc.index.get(id).is_some_and(public_visibility)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect()
}

fn public_visibility(item: &RustdocItem) -> bool {
    item.visibility.is_null() || item.visibility.as_str() == Some("public")
}

#[cfg(test)]
mod tests;

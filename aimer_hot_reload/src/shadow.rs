//! Deterministic preparation and analysis of standalone hot-reload projects.

mod discovery;
mod fingerprint;
mod guest;
mod manifest;
mod mirror;
mod transform;

use std::fmt;
use std::path::{Path, PathBuf};

pub use discovery::{RootDiscovery, SourceSpan, SourceType, StableIdentity};
pub use fingerprint::{AstFingerprint, fingerprint_expression, fingerprint_source};

/// Canonical local crates linked into a generated portable guest shadow.
///
/// The caller supplies both paths so shadow generation never assumes a
/// repository layout. Each path must name a local Cargo package directory;
/// preparation canonicalizes it and rejects missing or non-directory values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShadowGuestConfig {
    aimer_root: Option<PathBuf>,
    wasm_guest_root: Option<PathBuf>,
    portable_webbrowser_root: Option<PathBuf>,
    portable_reqwest_root: Option<PathBuf>,
}

impl ShadowGuestConfig {
    /// Creates an empty configuration completed through builder methods.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the local root of the umbrella `aimer` crate.
    #[inline]
    pub fn aimer_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.aimer_root = Some(root.into());
        self
    }

    /// Sets the local root of the `aimer_wasm_guest` crate.
    #[inline]
    pub fn wasm_guest_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.wasm_guest_root = Some(root.into());
        self
    }

    /// Sets the local guest-only replacement for the browser-opening crate.
    ///
    /// Portable guests cannot import browser globals directly. When an
    /// application depends on `webbrowser`, shadow generation can replace it
    /// with this no-browser implementation while native builds retain the
    /// application's original dependency.
    #[inline]
    pub fn portable_webbrowser_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.portable_webbrowser_root = Some(root.into());
        self
    }

    /// Sets the guest-only replacement for the network client crate.
    ///
    /// Native applications keep their ordinary `reqwest` dependency. A
    /// portable hot-reload guest instead routes its bounded GET surface
    /// through Aimer's single capability-call import, so wasm-bindgen browser
    /// imports cannot leak into the interpreted module.
    #[inline]
    pub fn portable_reqwest_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.portable_reqwest_root = Some(root.into());
        self
    }

    fn canonicalize(&self) -> Result<CanonicalGuestConfig, ShadowError> {
        Ok(CanonicalGuestConfig {
            aimer_root: canonical_crate_root(self.aimer_root.as_deref(), "aimer")?,
            wasm_guest_root: canonical_crate_root(
                self.wasm_guest_root.as_deref(),
                "aimer_wasm_guest",
            )?,
            portable_webbrowser_root: self
                .portable_webbrowser_root
                .as_deref()
                .map(|root| canonical_crate_root(Some(root), "aimer_portable_webbrowser"))
                .transpose()?,
            portable_reqwest_root: self
                .portable_reqwest_root
                .as_deref()
                .map(|root| canonical_crate_root(Some(root), "aimer_portable_reqwest"))
                .transpose()?,
        })
    }
}

pub(crate) struct CanonicalGuestConfig {
    pub aimer_root: PathBuf,
    pub wasm_guest_root: PathBuf,
    pub portable_webbrowser_root: Option<PathBuf>,
    pub portable_reqwest_root: Option<PathBuf>,
}

fn canonical_crate_root(root: Option<&Path>, name: &str) -> Result<PathBuf, ShadowError> {
    let root = root.ok_or_else(|| {
        ShadowError::new(
            ShadowErrorKind::Manifest,
            format!("local {name} crate root is required for guest generation"),
        )
    })?;
    let canonical = std::fs::canonicalize(root).map_err(|error| {
        ShadowError::new(
            ShadowErrorKind::Io,
            format!(
                "failed to resolve local {name} crate {}: {error}",
                root.display()
            ),
        )
    })?;
    if !canonical.is_dir() || !canonical.join("Cargo.toml").is_file() {
        return Err(ShadowError::new(
            ShadowErrorKind::Manifest,
            format!(
                "local {name} crate root is not a Cargo package: {}",
                canonical.display()
            ),
        ));
    }
    Ok(canonical)
}

/// Resource limits applied while preparing and analysing a shadow project.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShadowLimits {
    /// Maximum number of regular files copied into the shadow.
    pub max_files: usize,
    /// Maximum total number of bytes copied into the shadow.
    pub max_total_bytes: u64,
    /// Maximum number of bytes accepted for one file.
    pub max_file_bytes: u64,
    /// Maximum number of direct local calls followed from `#[aimer::main]`.
    pub max_ast_call_depth: usize,
}

impl Default for ShadowLimits {
    fn default() -> Self {
        Self {
            max_files: 16_384,
            max_total_bytes: 512 * 1024 * 1024,
            max_file_bytes: 16 * 1024 * 1024,
            max_ast_call_depth: 64,
        }
    }
}

impl ShadowLimits {
    /// Sets the maximum number of mirrored files.
    #[inline]
    pub const fn max_files(mut self, max_files: usize) -> Self {
        self.max_files = max_files;
        self
    }

    /// Sets the maximum total mirrored byte count.
    #[inline]
    pub const fn max_total_bytes(mut self, max_total_bytes: u64) -> Self {
        self.max_total_bytes = max_total_bytes;
        self
    }

    /// Sets the maximum byte count accepted for one file.
    #[inline]
    pub const fn max_file_bytes(mut self, max_file_bytes: u64) -> Self {
        self.max_file_bytes = max_file_bytes;
        self
    }

    /// Sets the maximum direct-call analysis depth.
    #[inline]
    pub const fn max_ast_call_depth(mut self, max_ast_call_depth: usize) -> Self {
        self.max_ast_call_depth = max_ast_call_depth;
        self
    }
}

/// A fully prepared standalone application shadow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowProject {
    root: PathBuf,
    manifest: PathBuf,
    discovery: RootDiscovery,
    source_types: Vec<SourceType>,
}

impl ShadowProject {
    /// Returns the canonical output root containing the shadow project.
    #[inline]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the rewritten standalone Cargo manifest.
    #[inline]
    pub fn manifest(&self) -> &Path {
        &self.manifest
    }

    /// Returns the unique application root discovered from `#[aimer::main]`.
    #[inline]
    pub fn discovery(&self) -> &RootDiscovery {
        &self.discovery
    }

    /// Returns source structs in stable package/module/type order.
    #[inline]
    pub fn source_types(&self) -> &[SourceType] {
        &self.source_types
    }
}

/// Stable category for a shadow preparation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShadowErrorKind {
    /// A filesystem operation failed.
    Io,
    /// A path or symbolic link escaped the accepted roots.
    PathEscape,
    /// The output overlaps the source project.
    OutputRecursion,
    /// A configured resource bound was exceeded.
    LimitExceeded,
    /// A Cargo manifest was missing or malformed.
    Manifest,
    /// Rust source could not be parsed.
    MalformedSource,
    /// Two module declarations own the same source or one module has two files.
    DuplicateModule,
    /// The application entry point or root flow is ambiguous.
    AmbiguousFlow,
    /// A direct local call could not be resolved statically.
    UnresolvedFlow,
    /// The root flow uses dynamic dispatch or another non-static call.
    DynamicFlow,
    /// No complete application builder chain was found.
    MissingFlow,
}

/// Failure to prepare or statically analyse a shadow project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowError {
    kind: ShadowErrorKind,
    message: String,
    span: Option<SourceSpan>,
}

impl ShadowError {
    pub(crate) fn new(kind: ShadowErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            span: None,
        }
    }

    pub(crate) fn at(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }

    /// Returns the stable error category.
    #[inline]
    pub const fn kind(&self) -> ShadowErrorKind {
        self.kind
    }

    /// Returns the source location associated with the failure, when available.
    #[inline]
    pub fn span(&self) -> Option<&SourceSpan> {
        self.span.as_ref()
    }

    /// Returns the human-readable diagnostic message without source context.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ShadowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ShadowError {}

/// Mirrors, rewrites, and analyses one application as a standalone project.
///
/// `project_root` must contain `Cargo.toml`. `output_root` must not overlap the
/// source tree and is replaced deterministically after all source paths and
/// limits have been validated. Symbolic links are rejected rather than
/// followed. Relative dependency paths are canonicalized before the source is
/// copied, and workspace-inherited dependencies are materialized in the
/// output manifest.
///
/// After discovery, eligible source-defined structs in the copied project gain
/// hidden reflection and portable-state implementations. The application files
/// under `project_root` are never rewritten, and preparing the same inputs
/// repeatedly produces identical shadow bytes.
pub fn prepare_shadow_project(
    project_root: &Path,
    output_root: &Path,
    limits: ShadowLimits,
) -> Result<ShadowProject, ShadowError> {
    let rewritten = manifest::rewrite(project_root)?;
    let mirror = mirror::validate_and_copy(project_root, output_root, limits, &rewritten.bytes)?;
    let manifest = mirror.output_root.join("Cargo.toml");
    let (discovery, source_types) = discovery::discover(
        &mirror.source_root,
        &rewritten.package,
        &rewritten.value,
        limits.max_ast_call_depth,
    )?;
    transform::transform(&mirror.output_root, &rewritten.package, &rewritten.value)?;
    Ok(ShadowProject {
        root: mirror.output_root,
        manifest,
        discovery,
        source_types,
    })
}

/// Prepares a standalone, dependency-linkable portable guest shadow.
///
/// This performs ordinary bounded shadow preparation, then rewrites only the
/// copied package as a `cdylib`/`rlib`, links the caller-supplied local guest
/// crates, enables `aimer/portable-guest`, suppresses binary auto-discovery,
/// and emits the generated root factory and [`aimer_wasm_guest::GuestProgram`]
/// adapter. The source project remains byte-for-byte unchanged.
pub fn prepare_guest_shadow_project(
    project_root: &Path,
    output_root: &Path,
    limits: ShadowLimits,
    guest_config: ShadowGuestConfig,
) -> Result<ShadowProject, ShadowError> {
    let guest_config = guest_config.canonicalize()?;
    let source_root = std::fs::canonicalize(project_root).map_err(|error| {
        ShadowError::new(
            ShadowErrorKind::Io,
            format!(
                "failed to resolve source project {}: {error}",
                project_root.display()
            ),
        )
    })?;
    let mut rewritten = manifest::rewrite(&source_root)?;
    let (discovery, source_types) = discovery::discover(
        &source_root,
        &rewritten.package,
        &rewritten.value,
        limits.max_ast_call_depth,
    )?;
    manifest::enable_guest(&mut rewritten, &source_root, &discovery, &guest_config)?;
    let mirror = mirror::validate_and_copy(&source_root, output_root, limits, &rewritten.bytes)?;
    let manifest = mirror.output_root.join("Cargo.toml");
    transform::transform(&mirror.output_root, &rewritten.package, &rewritten.value)?;
    guest::generate(
        &source_root,
        &mirror.output_root,
        &rewritten.package,
        &discovery,
        &source_types,
    )?;
    Ok(ShadowProject {
        root: mirror.output_root,
        manifest,
        discovery,
        source_types,
    })
}

#[cfg(test)]
mod tests;

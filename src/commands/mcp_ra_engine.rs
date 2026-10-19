//! In-process rust-analyzer IDE engine used by the MCP adapter.
//!
//! This module speaks directly to the `ide` and `load-cargo` crates from the
//! Aimer rust-analyzer fork. MCP keeps its stable JSON surface; no LSP client
//! or rust-analyzer executable is involved.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use ra_ap_hir::{ClosureStyle, PrefixKind};
use ra_ap_ide::{
    Analysis, AnalysisHost, AssistConfig, AssistResolveStrategy, CallHierarchyConfig,
    CallableSnippets, CompletionConfig, CompletionFieldsToResolve, DiagnosticCode,
    DiagnosticsConfig, DiscriminantHints, FileId, FilePosition, FileRange, FileStructureConfig,
    FindAllRefsConfig, GenericParameterHints, GotoDefinitionConfig, GotoImplementationConfig,
    HighlightRelatedConfig, HoverConfig, HoverDocFormat, InlayFieldsToResolve, InlayHintPosition,
    InlayHintsConfig, InlayKind, LifetimeElisionHints, NavigationTarget, RenameConfig, Severity,
    StructureNode, StructureNodeKind, SymbolKind, TextRange, TextSize, TypeHintsPlacement,
};
use ra_ap_ide_db::{
    ChangeWithProcMacros,
    imports::insert_use::{ImportGranularity, InsertUseConfig},
    ra_fixture::RaFixtureConfig,
};
use ra_ap_load_cargo::{LoadCargoConfig, ProcMacroServerChoice, load_workspace_at};
use ra_ap_project_model::{CargoConfig, RustLibSource};
use ra_ap_vfs::{AbsPathBuf, FileExcluded, Vfs, VfsPath};
use serde_json::{Value, json};
use tempfile::TempDir;

include!(concat!(env!("OUT_DIR"), "/aimer_proc_macro_server.rs"));

mod analyzer;
mod positions;

const MAX_ENGINE_RESULTS: usize = 200;
const MAX_IMPLEMENTATION_RESULTS: usize = 5_000;

pub(crate) struct EmbeddedRustAnalyzer {
    root: PathBuf,
    host: AnalysisHost,
    vfs: Vfs,
    _proc_macro_client: Option<ra_ap_proc_macro_api::ProcMacroClient>,
    _proc_macro_helper_tempdir: Option<TempDir>,
    _watcher: WorkspaceWatcher,
    events: Receiver<notify::Result<Event>>,
    proc_macro_server: Option<PathBuf>,
}

#[allow(dead_code)] // The values keep the selected backend alive until the engine drops.
enum WorkspaceWatcher {
    Native(notify::RecommendedWatcher),
    Polling(notify::PollWatcher),
}

fn forward_events(
    sender: mpsc::Sender<notify::Result<Event>>,
) -> impl FnMut(notify::Result<Event>) + Send + 'static {
    move |event| {
        let _ = sender.send(event);
    }
}

fn resolve_proc_macro_server(
    configured: &Option<PathBuf>,
) -> anyhow::Result<(Option<PathBuf>, Option<TempDir>)> {
    if let Some(path) = configured {
        let path = path
            .canonicalize()
            .with_context(|| format!("resolving proc-macro helper '{}'", path.display()))?;
        anyhow::ensure!(
            path.is_file(),
            "proc-macro helper '{}' is not a file",
            path.display()
        );
        return Ok((Some(path), None));
    }
    let helper_name = format!(
        "rust-analyzer-proc-macro-srv{}",
        std::env::consts::EXE_SUFFIX
    );
    if let Some(helper_bytes) = BUNDLED_PROC_MACRO_SERVER {
        let helper_tempdir = tempfile::Builder::new()
            .prefix("aimer-proc-macro-")
            .tempdir()
            .context("creating a private directory for the bundled proc-macro helper")?;
        let helper_path = helper_tempdir.path().join(&helper_name);
        fs::write(&helper_path, helper_bytes).with_context(|| {
            format!(
                "extracting bundled proc-macro helper to {}",
                helper_path.display()
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&helper_path, fs::Permissions::from_mode(0o700))
                .with_context(|| format!("making {} executable", helper_path.display()))?;
        }
        let helper_path = helper_path
            .canonicalize()
            .with_context(|| format!("resolving extracted helper {}", helper_path.display()))?;
        return Ok((Some(helper_path), Some(helper_tempdir)));
    }

    let executable = std::env::current_exe().context("locating Aimer executable")?;
    let Some(candidate) = executable
        .parent()
        .map(|parent| parent.join(helper_name))
        .filter(|candidate| candidate.is_file())
    else {
        return Ok((None, None));
    };
    Ok((candidate.canonicalize().ok(), None))
}

fn uri_to_path(uri: &str) -> anyhow::Result<PathBuf> {
    reqwest::Url::parse(uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| anyhow!("request contains an invalid file URI"))
}

fn text_size_to_usize(size: TextSize) -> usize {
    u32::from(size) as usize
}

fn text_range_to_lsp(source: &str, range: TextRange) -> Value {
    json!({
        "start": offset_to_lsp(source, text_size_to_usize(range.start())),
        "end": offset_to_lsp(source, text_size_to_usize(range.end()))
    })
}

fn offset_to_lsp(source: &str, offset: usize) -> Value {
    let offset = offset.min(source.len());
    let offset = (0..=offset)
        .rev()
        .find(|index| source.is_char_boundary(*index))
        .unwrap_or(0);
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let character = prefix
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .encode_utf16()
        .count();
    json!({ "line": line, "character": character })
}

fn lsp_range_to_text(source: &str, range: &Value) -> anyhow::Result<TextRange> {
    let start = source_offset(source, range.get("start").unwrap_or(&Value::Null))?;
    let end = source_offset(source, range.get("end").unwrap_or(&Value::Null))?;
    anyhow::ensure!(start <= end, "LSP range end precedes its start");
    Ok(TextRange::new(
        TextSize::from(start as u32),
        TextSize::from(end as u32),
    ))
}

fn source_offset(source: &str, position: &Value) -> anyhow::Result<usize> {
    let line = position.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let character = position
        .get("character")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let mut offset = 0;
    for (index, line_text) in source.split_inclusive('\n').enumerate() {
        if index == line {
            let mut units = 0;
            for (byte, ch) in line_text.char_indices() {
                if units >= character {
                    return Ok(offset + byte);
                }
                units += ch.len_utf16();
            }
            let content = line_text.trim_end_matches(['\r', '\n']);
            if character == content.encode_utf16().count() {
                return Ok(offset + content.len());
            }
            bail!("LSP position is outside the source line");
        }
        offset += line_text.len();
    }
    if line == source.split('\n').count().saturating_sub(1) && character == 0 {
        return Ok(source.len());
    }
    bail!("LSP line is outside the source file")
}

fn symbol_kind_number(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Module | SymbolKind::CrateRoot | SymbolKind::ToolModule => 2,
        SymbolKind::Struct | SymbolKind::Union | SymbolKind::Impl => 23,
        SymbolKind::Enum => 10,
        SymbolKind::Trait => 11,
        SymbolKind::Function => 12,
        SymbolKind::Method => 6,
        SymbolKind::Field => 8,
        SymbolKind::Const | SymbolKind::ConstParam => 14,
        SymbolKind::Static | SymbolKind::Local | SymbolKind::ValueParam | SymbolKind::SelfParam => {
            13
        }
        SymbolKind::TypeParam | SymbolKind::LifetimeParam => 26,
        SymbolKind::Variant => 22,
        SymbolKind::TypeAlias | SymbolKind::SelfType => 5,
        SymbolKind::Macro
        | SymbolKind::ProcMacro
        | SymbolKind::Derive
        | SymbolKind::DeriveHelper
        | SymbolKind::Attribute
        | SymbolKind::BuiltinAttr => 12,
        SymbolKind::Label | SymbolKind::InlineAsmRegOrRegClass => 25,
    }
}

fn structure_kind_number(kind: StructureNodeKind) -> u8 {
    match kind {
        StructureNodeKind::SymbolKind(kind) => symbol_kind_number(kind),
        StructureNodeKind::ExternBlock => 2,
        StructureNodeKind::Region => 3,
    }
}

fn completion_kind_number(kind: ra_ap_ide::CompletionItemKind) -> u8 {
    match kind {
        ra_ap_ide::CompletionItemKind::SymbolKind(kind) => match kind {
            SymbolKind::Method => 2,
            SymbolKind::Function | SymbolKind::Macro | SymbolKind::ProcMacro => 3,
            SymbolKind::Field => 5,
            SymbolKind::Const | SymbolKind::Static => 21,
            SymbolKind::Enum => 13,
            SymbolKind::Struct | SymbolKind::Union => 22,
            SymbolKind::Trait => 8,
            SymbolKind::Module | SymbolKind::CrateRoot => 9,
            SymbolKind::Variant => 20,
            SymbolKind::TypeParam | SymbolKind::LifetimeParam | SymbolKind::TypeAlias => 25,
            _ => 6,
        },
        ra_ap_ide::CompletionItemKind::Keyword => 14,
        ra_ap_ide::CompletionItemKind::Snippet => 15,
        ra_ap_ide::CompletionItemKind::BuiltinType => 25,
        ra_ap_ide::CompletionItemKind::Binding | ra_ap_ide::CompletionItemKind::InferredType => 6,
        ra_ap_ide::CompletionItemKind::UnresolvedReference
        | ra_ap_ide::CompletionItemKind::Expression => 1,
    }
}

fn inlay_kind_number(kind: InlayKind) -> u8 {
    match kind {
        InlayKind::Parameter => 2,
        InlayKind::GenericParamList | InlayKind::Lifetime | InlayKind::GenericParameter => 2,
        _ => 1,
    }
}

fn severity_number(severity: Severity) -> u8 {
    match severity {
        Severity::Error => 1,
        Severity::Warning | Severity::WeakWarning => 2,
        Severity::Allow => 3,
    }
}

fn diagnostic_source(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::RustcHardError(_) | DiagnosticCode::RustcLint(_) => "rustc",
        DiagnosticCode::Clippy(_) => "clippy",
        DiagnosticCode::SyntaxError | DiagnosticCode::Ra(_, _) | DiagnosticCode::RaLint(_, _) => {
            "rust-analyzer"
        }
    }
}

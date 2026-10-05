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

const MAX_ENGINE_RESULTS: usize = 200;

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

impl EmbeddedRustAnalyzer {
    pub(crate) fn load(
        root: PathBuf,
        config: &aimer_hot_reload::config::RustAnalyzerConfig,
        report_progress: &(dyn Fn(String) + Sync),
    ) -> anyhow::Result<Self> {
        let mut cargo_config = CargoConfig::default();
        // `CargoConfig::default()` disables the sysroot entirely. Load its
        // sources so core/std and proc_macro targets enter RA's crate graph.
        cargo_config.sysroot = Some(RustLibSource::Discover);
        // Match rust-analyzer's default Cargo model: index every target and
        // treat `#[cfg(test)]` code as part of the workspace analysis.
        cargo_config.all_targets = true;
        cargo_config.set_test = true;
        cargo_config.extra_env.insert(
            "CARGO_NET_OFFLINE".to_owned(),
            (!config.allow_network).then(|| "true".to_owned()),
        );

        let (proc_macro_server, proc_macro_helper_tempdir) =
            resolve_proc_macro_server(&config.proc_macro_server_path)?;
        let with_proc_macro_server = proc_macro_server
            .as_ref()
            .map_or(ProcMacroServerChoice::None, |path| {
                ProcMacroServerChoice::Explicit(AbsPathBuf::assert_utf8(path.clone()))
            });
        let load_config = LoadCargoConfig {
            load_out_dirs_from_check: true,
            with_proc_macro_server,
            // Avoid eagerly type-checking the entire standard library on the
            // MCP startup path; queries fill the RA caches on demand.
            prefill_caches: false,
            num_worker_threads: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(4)
                .clamp(1, 8),
            proc_macro_processes: 1,
        };
        report_progress(format!(
            "Loading {} into the embedded Rust analysis engine.",
            root.display()
        ));
        let (database, mut vfs, proc_macro_client) =
            load_workspace_at(&root, &cargo_config, &load_config, report_progress)
                .with_context(|| format!("loading Rust workspace '{}'", root.display()))?;
        if proc_macro_server.is_some() && proc_macro_client.is_none() {
            bail!(
                "the configured proc-macro helper could not start; rebuild it with the same Rust toolchain and Aimer rust-analyzer release"
            );
        }
        let host = AnalysisHost::with_database(database);
        let (event_sender, events) = mpsc::channel();
        let native_watcher = notify::recommended_watcher(forward_events(event_sender.clone()))
            .context("creating Rust workspace file watcher")
            .and_then(|mut watcher| {
                watcher
                    .watch(&root, RecursiveMode::Recursive)
                    .with_context(|| format!("watching Rust workspace '{}'", root.display()))?;
                Ok(WorkspaceWatcher::Native(watcher))
            });
        let watcher = match native_watcher {
            Ok(watcher) => watcher,
            Err(native_error) => {
                tracing::debug!(%native_error, "native workspace watcher unavailable; falling back to polling");
                report_progress(format!(
                    "Native file watching is unavailable ({native_error}); using polling for workspace updates."
                ));
                let mut watcher = notify::PollWatcher::new(
                    forward_events(event_sender),
                    notify::Config::default().with_poll_interval(Duration::from_secs(2)),
                )
                .context("creating polling Rust workspace watcher")?;
                watcher
                    .watch(&root, RecursiveMode::Recursive)
                    .with_context(|| {
                        format!(
                            "watching Rust workspace '{}' with polling after native watcher failed: {native_error}",
                            root.display()
                        )
                    })?;
                WorkspaceWatcher::Polling(watcher)
            }
        };
        let _ = vfs.take_changes();
        report_progress(if proc_macro_server.is_some() {
            "Embedded Rust analysis is ready; the matching proc-macro helper is enabled.".into()
        } else {
            "Embedded Rust analysis is ready; no matching proc-macro helper was found.".into()
        });
        Ok(Self {
            root,
            host,
            vfs,
            _proc_macro_client: proc_macro_client,
            _proc_macro_helper_tempdir: proc_macro_helper_tempdir,
            _watcher: watcher,
            events,
            proc_macro_server,
        })
    }

    pub(crate) fn proc_macro_server(&self) -> Option<&Path> {
        self.proc_macro_server.as_deref()
    }

    /// Apply changes to already indexed Rust files. Structural and manifest
    /// changes request a workspace reload so the crate graph stays coherent.
    pub(crate) fn poll_file_events(&mut self) -> anyhow::Result<bool> {
        let mut reload = false;
        loop {
            let event = match self.events.try_recv() {
                Ok(Ok(event)) => event,
                Ok(Err(error)) => {
                    tracing::debug!(%error, "embedded RA workspace watcher reported an error");
                    reload = true;
                    continue;
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            let structural = matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_));
            for path in event.paths {
                if path.strip_prefix(&self.root).ok().is_some_and(|relative| {
                    relative.components().any(|component| {
                        matches!(component, std::path::Component::Normal(name) if name == "target" || name == ".git" || name == ".aimer")
                    })
                }) {
                    continue;
                }
                if path
                    .file_name()
                    .is_some_and(|name| name == "Cargo.toml" || name == "build.rs")
                {
                    reload = true;
                    continue;
                }
                if path.extension().is_some_and(|extension| extension == "rs") {
                    if structural || !path.is_file() {
                        reload = true;
                    } else if let Ok(source) = std::fs::read_to_string(&path) {
                        if self.update_indexed_file(&path, &source).is_err() {
                            reload = true;
                        }
                    }
                }
            }
        }
        Ok(reload)
    }

    pub(crate) fn update_document(&mut self, path: &Path, source: &str) -> anyhow::Result<()> {
        let path = self.resolve_path(path)?;
        let vfs_path = VfsPath::new_real_path(path.to_string_lossy().into_owned());
        let Some((_, excluded)) = self.vfs.file_id(&vfs_path) else {
            bail!(
                "Rust file '{}' is not part of the loaded workspace; reload the workspace",
                path.display()
            );
        };
        if excluded == FileExcluded::Yes {
            bail!("Rust file '{}' is excluded from analysis", path.display());
        }
        self.update_indexed_file(&path, source)
    }

    fn update_indexed_file(&mut self, path: &Path, source: &str) -> anyhow::Result<()> {
        let path = self.resolve_path(path)?;
        let vfs_path = VfsPath::new_real_path(path.to_string_lossy().into_owned());
        self.vfs
            .set_file_contents(vfs_path.clone(), Some(source.as_bytes().to_vec()));
        let (file_id, excluded) = self
            .vfs
            .file_id(&vfs_path)
            .ok_or_else(|| anyhow!("file disappeared from the Rust workspace"))?;
        if excluded == FileExcluded::Yes {
            bail!("Rust file '{}' is excluded from analysis", path.display());
        }
        let mut change = ChangeWithProcMacros::default();
        change.change_file(file_id, Some(source.to_owned()));
        self.host.apply_change(change);
        let _ = self.vfs.take_changes();
        Ok(())
    }

    pub(crate) fn diagnostics(&self, file_filter: Option<&Path>) -> anyhow::Result<Value> {
        let analysis = self.host.analysis();
        let config = DiagnosticsConfig::test_sample();
        let filter = file_filter
            .map(|path| self.file_id_for_path(path))
            .transpose()?;
        let files = self
            .vfs
            .iter()
            .filter(|(file_id, path)| {
                let absolute = PathBuf::from(self.vfs.file_path(*file_id).to_string());
                absolute.starts_with(&self.root)
                    && path
                        .name_and_extension()
                        .is_some_and(|(_, extension)| extension == Some("rs"))
            })
            .map(|(file_id, _)| file_id)
            .filter(|file_id| filter.is_none_or(|filter| filter == *file_id))
            .collect::<Vec<_>>();
        let mut diagnostics = Vec::new();
        for file_id in files {
            for diagnostic in analysis
                .full_diagnostics(&config, AssistResolveStrategy::None, file_id)
                .map_err(|error| anyhow!("Rust diagnostics were cancelled: {error:?}"))?
            {
                let source = analysis
                    .file_text(diagnostic.range.file_id)
                    .map_err(|error| {
                        anyhow!("reading diagnostic source was cancelled: {error:?}")
                    })?;
                diagnostics.push(json!({
                    "file": self.relative_file(diagnostic.range.file_id),
                    "range": text_range_to_lsp(&source, diagnostic.range.range),
                    "severity": severity_number(diagnostic.severity),
                    "source": diagnostic_source(diagnostic.code),
                    "code": diagnostic.code.as_str(),
                    "message": diagnostic.message
                }));
            }
        }
        diagnostics.sort_by(|left, right| {
            left["file"]
                .as_str()
                .cmp(&right["file"].as_str())
                .then_with(|| {
                    left["range"]["start"]["line"]
                        .as_u64()
                        .cmp(&right["range"]["start"]["line"].as_u64())
                })
                .then_with(|| left["code"].as_str().cmp(&right["code"].as_str()))
        });
        let truncated = diagnostics.len() > MAX_ENGINE_RESULTS;
        diagnostics.truncate(MAX_ENGINE_RESULTS);
        Ok(json!({ "diagnostics": diagnostics, "truncated": truncated }))
    }

    pub(crate) fn request(&self, method: &str, params: &Value) -> anyhow::Result<Value> {
        let analysis = self.host.analysis();
        match method {
            "workspace/symbol" => self.workspace_symbols(&analysis, params),
            "textDocument/documentSymbol" => self.document_symbols(&analysis, params),
            "textDocument/definition" => self.navigation_request(&analysis, params, "definition"),
            "textDocument/declaration" => self.navigation_request(&analysis, params, "declaration"),
            "textDocument/implementation" => {
                self.navigation_request(&analysis, params, "implementation")
            }
            "textDocument/typeDefinition" => {
                self.navigation_request(&analysis, params, "typeDefinition")
            }
            "textDocument/references" => self.references(&analysis, params),
            "textDocument/hover" => self.hover(&analysis, params),
            "textDocument/signatureHelp" => self.signature_help(&analysis, params),
            "textDocument/prepareRename" => self.prepare_rename(&analysis, params),
            "textDocument/rename" => self.rename(&analysis, params),
            "textDocument/completion" => self.completions(&analysis, params),
            "textDocument/documentHighlight" => self.document_highlights(&analysis, params),
            "textDocument/inlayHint" => self.inlay_hints(&analysis, params),
            "textDocument/codeAction" => self.code_actions(&analysis, params),
            "textDocument/codeLens" => self.code_lenses(&analysis, params),
            "codeLens/resolve" | "codeAction/resolve" => Ok(params.clone()),
            "rust-analyzer/expandMacro" => self.expand_macro(&analysis, params),
            "textDocument/prepareCallHierarchy" => self.prepare_call_hierarchy(&analysis, params),
            "callHierarchy/incomingCalls" => self.call_hierarchy_calls(&analysis, params, true),
            "callHierarchy/outgoingCalls" => self.call_hierarchy_calls(&analysis, params, false),
            "rust-analyzer/reloadWorkspace" => Ok(json!({ "status": "reload_on_next_request" })),
            method => bail!("embedded rust-analyzer does not support request '{method}'"),
        }
    }

    fn workspace_symbols(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let symbols = analysis
            .symbol_search(
                ra_ap_ide::Query::new(query.to_owned()),
                MAX_ENGINE_RESULTS + 1,
            )
            .map_err(|error| anyhow!("workspace symbol search was cancelled: {error:?}"))?;
        Ok(Value::Array(
            symbols
                .into_iter()
                .take(MAX_ENGINE_RESULTS)
                .map(|target| {
                    let (uri, range) = self.navigation_location(analysis, &target)?;
                    let mut item = json!({
                        "name": target.name.to_string(),
                        "kind": target.kind.map(symbol_kind_number).unwrap_or(13),
                        "location": { "uri": uri, "range": range }
                    });
                    if let Some(container) = target.container_name {
                        item["containerName"] = json!(container.to_string());
                    }
                    Ok(item)
                })
                .collect::<anyhow::Result<Vec<_>>>()?,
        ))
    }

    fn document_symbols(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let file_id = self.file_id_for_params(params)?;
        let source = self.file_text(analysis, file_id)?;
        let nodes = analysis
            .file_structure(
                &FileStructureConfig {
                    exclude_locals: false,
                },
                file_id,
            )
            .map_err(|error| anyhow!("document symbol search was cancelled: {error:?}"))?;
        let mut children = vec![Vec::<usize>::new(); nodes.len()];
        let mut roots = Vec::new();
        for (index, node) in nodes.iter().enumerate() {
            if let Some(parent) = node.parent.filter(|parent| *parent < nodes.len()) {
                children[parent].push(index);
            } else {
                roots.push(index);
            }
        }
        fn node_json(
            index: usize,
            nodes: &[StructureNode],
            children: &[Vec<usize>],
            source: &str,
        ) -> Value {
            let node = &nodes[index];
            let mut value = json!({
                "name": node.label,
                "kind": structure_kind_number(node.kind),
                "range": text_range_to_lsp(source, node.node_range),
                "selectionRange": text_range_to_lsp(source, node.navigation_range),
                "deprecated": node.deprecated
            });
            if let Some(detail) = &node.detail {
                value["detail"] = json!(detail);
            }
            if !children[index].is_empty() {
                value["children"] = Value::Array(
                    children[index]
                        .iter()
                        .map(|child| node_json(*child, nodes, children, source))
                        .collect(),
                );
            }
            value
        }
        Ok(Value::Array(
            roots
                .into_iter()
                .map(|index| node_json(index, &nodes, &children, &source))
                .collect(),
        ))
    }

    fn navigation_request(
        &self,
        analysis: &Analysis,
        params: &Value,
        operation: &str,
    ) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let targets = match operation {
            "definition" => analysis.goto_definition(
                position,
                &GotoDefinitionConfig {
                    ra_fixture: RaFixtureConfig::default(),
                },
            ),
            "declaration" => analysis.goto_declaration(
                position,
                &GotoDefinitionConfig {
                    ra_fixture: RaFixtureConfig::default(),
                },
            ),
            "implementation" => analysis.goto_implementation(
                &GotoImplementationConfig {
                    filter_adjacent_derive_implementations: true,
                },
                position,
            ),
            "typeDefinition" => analysis.goto_type_definition(position),
            _ => unreachable!("validated navigation operation"),
        }
        .map_err(|error| anyhow!("Rust navigation was cancelled: {error:?}"))?
        .map(|result| result.info)
        .unwrap_or_default();
        let mut locations = Vec::new();
        for target in targets.into_iter().take(MAX_ENGINE_RESULTS) {
            let (uri, range) = self.navigation_location(analysis, &target)?;
            let (_, selection_range) = self.navigation_selection(analysis, &target)?;
            locations.push(json!({
                "targetUri": uri,
                "targetRange": range,
                "targetSelectionRange": selection_range
            }));
        }
        Ok(Value::Array(locations))
    }

    fn references(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let found = analysis
            .find_all_refs(
                position,
                &FindAllRefsConfig {
                    search_scope: None,
                    ra_fixture: RaFixtureConfig::default(),
                    exclude_imports: false,
                    exclude_tests: false,
                },
            )
            .map_err(|error| anyhow!("Rust reference search was cancelled: {error:?}"))?;
        let include_declaration = params
            .pointer("/context/includeDeclaration")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut locations = Vec::new();
        for result in found.into_iter().flatten() {
            if include_declaration {
                if let Some(declaration) = result.declaration {
                    let (uri, range) = self.navigation_selection(analysis, &declaration.nav)?;
                    locations.push(json!({ "uri": uri, "range": range }));
                }
            }
            for (file_id, references) in result.references {
                let source = self.file_text(analysis, file_id)?;
                let uri = self.file_uri(file_id)?;
                for (range, _) in references {
                    locations
                        .push(json!({ "uri": uri, "range": text_range_to_lsp(&source, range) }));
                }
            }
        }
        locations.sort_by(|left, right| {
            left["uri"]
                .as_str()
                .cmp(&right["uri"].as_str())
                .then_with(|| {
                    left["range"]["start"]["line"]
                        .as_u64()
                        .cmp(&right["range"]["start"]["line"].as_u64())
                })
                .then_with(|| {
                    left["range"]["start"]["character"]
                        .as_u64()
                        .cmp(&right["range"]["start"]["character"].as_u64())
                })
        });
        locations.dedup();
        locations.truncate(MAX_ENGINE_RESULTS);
        Ok(Value::Array(locations))
    }

    fn hover(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let config = HoverConfig {
            links_in_hover: false,
            memory_layout: None,
            documentation: true,
            keywords: true,
            format: HoverDocFormat::Markdown,
            max_trait_assoc_items_count: Some(30),
            max_fields_count: Some(30),
            max_enum_variants_count: Some(30),
            max_subst_ty_len: ra_ap_ide::SubstTyLen::LimitTo(500),
            show_drop_glue: false,
            ra_fixture: RaFixtureConfig::default(),
        };
        let range = TextRange::empty(position.offset);
        let hover = analysis
            .hover(
                &config,
                FileRange {
                    file_id: position.file_id,
                    range,
                },
            )
            .map_err(|error| anyhow!("Rust hover was cancelled: {error:?}"))?;
        let Some(hover) = hover else {
            return Ok(Value::Null);
        };
        let source = self.file_text(analysis, position.file_id)?;
        Ok(json!({
            "contents": { "kind": "markdown", "value": hover.info.markup.as_str() },
            "range": text_range_to_lsp(&source, hover.range)
        }))
    }

    fn signature_help(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let Some(help) = analysis
            .signature_help(position)
            .map_err(|error| anyhow!("Rust signature help was cancelled: {error:?}"))?
        else {
            return Ok(Value::Null);
        };
        let parameters = help
            .parameter_labels()
            .map(|label| json!({ "label": label }))
            .collect::<Vec<_>>();
        Ok(json!({
            "signatures": [{
                "label": help.signature,
                "documentation": help.doc.as_ref().map(|doc| doc.as_str()),
                "parameters": parameters
            }],
            "activeSignature": 0,
            "activeParameter": help.active_parameter
        }))
    }

    fn prepare_rename(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let prepared = analysis
            .prepare_rename(position)
            .map_err(|error| anyhow!("Rust rename preparation was cancelled: {error:?}"))?
            .map_err(|error| anyhow!("Rust symbol cannot be renamed: {error:?}"))?;
        Ok(
            json!({ "range": text_range_to_lsp(&self.file_text(analysis, position.file_id)?, prepared.range) }),
        )
    }

    fn rename(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let new_name = params
            .get("newName")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("rename request omitted newName"))?;
        let source_change = analysis
            .rename(
                position,
                new_name,
                &RenameConfig {
                    prefer_no_std: false,
                    prefer_prelude: true,
                    prefer_absolute: false,
                    show_conflicts: true,
                },
            )
            .map_err(|error| anyhow!("Rust rename was cancelled: {error:?}"))?
            .map_err(|error| anyhow!("Rust rename failed: {error:?}"))?;
        self.source_change_to_workspace_edit(analysis, &source_change)
    }

    fn completions(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let import_use = InsertUseConfig {
            granularity: ImportGranularity::Item,
            enforce_granularity: false,
            prefix_kind: PrefixKind::Plain,
            group: false,
            skip_glob_imports: false,
        };
        let config = CompletionConfig {
            enable_postfix_completions: true,
            enable_imports_on_the_fly: true,
            enable_self_on_the_fly: true,
            enable_auto_iter: true,
            enable_auto_await: true,
            enable_private_editable: true,
            enable_term_search: true,
            term_search_fuel: 400,
            full_function_signatures: true,
            callable: Some(CallableSnippets::FillArguments),
            add_colons_to_module: true,
            add_semicolon_to_unit: true,
            snippet_cap: None,
            insert_use: import_use,
            prefer_no_std: false,
            prefer_prelude: true,
            prefer_absolute: false,
            snippets: Vec::new(),
            limit: Some(MAX_ENGINE_RESULTS + 1),
            fields_to_resolve: CompletionFieldsToResolve::empty(),
            exclude_flyimport: Vec::new(),
            exclude_traits: &[],
            ra_fixture: RaFixtureConfig::default(),
        };
        let items = analysis
            .completions(&config, position, None)
            .map_err(|error| anyhow!("Rust completion was cancelled: {error:?}"))?
            .unwrap_or_default();
        let source = self.file_text(analysis, position.file_id)?;
        let mut results = Vec::new();
        for item in items.into_iter().take(MAX_ENGINE_RESULTS + 1) {
            let mut value = json!({
                "label": item.label.primary.to_string(),
                "kind": completion_kind_number(item.kind),
                "filterText": item.lookup.to_string(),
                "deprecated": item.deprecated,
                "detail": item.detail
            });
            if let Some(documentation) = item.documentation {
                value["documentation"] =
                    json!({ "kind": "markdown", "value": documentation.as_str() });
            }
            if let Some(indel) = item.text_edit.iter().next() {
                value["textEdit"] = json!({
                    "range": text_range_to_lsp(&source, indel.delete),
                    "newText": indel.insert
                });
            }
            results.push(value);
        }
        let truncated = results.len() > MAX_ENGINE_RESULTS;
        results.truncate(MAX_ENGINE_RESULTS);
        Ok(json!({ "isIncomplete": truncated, "items": results }))
    }

    fn document_highlights(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let result = analysis
            .highlight_related(
                HighlightRelatedConfig {
                    references: true,
                    ..Default::default()
                },
                position,
            )
            .map_err(|error| anyhow!("Rust document highlight was cancelled: {error:?}"))?;
        let source = self.file_text(analysis, position.file_id)?;
        Ok(Value::Array(
            result
                .unwrap_or_default()
                .into_iter()
                .map(|highlight| {
                    json!({
                        "range": text_range_to_lsp(&source, highlight.range),
                        "kind": 2
                    })
                })
                .collect(),
        ))
    }

    fn inlay_hints(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let file_id = self.file_id_for_params(params)?;
        let source = self.file_text(analysis, file_id)?;
        let range = params
            .get("range")
            .map(|range| lsp_range_to_text(&source, range))
            .transpose()?;
        let config = InlayHintsConfig {
            render_colons: true,
            type_hints: true,
            type_hints_placement: TypeHintsPlacement::Inline,
            sized_bound: false,
            discriminant_hints: DiscriminantHints::Never,
            parameter_hints: true,
            parameter_hints_for_missing_arguments: false,
            generic_parameter_hints: GenericParameterHints {
                type_hints: true,
                lifetime_hints: false,
                const_hints: false,
            },
            chaining_hints: true,
            adjustment_hints: ra_ap_ide::AdjustmentHints::Never,
            adjustment_hints_disable_reborrows: false,
            adjustment_hints_mode: ra_ap_ide::AdjustmentHintsMode::Prefix,
            adjustment_hints_hide_outside_unsafe: false,
            closure_return_type_hints: ra_ap_ide::ClosureReturnTypeHints::Never,
            closure_capture_hints: false,
            binding_mode_hints: false,
            implicit_drop_hints: false,
            implied_dyn_trait_hints: false,
            lifetime_elision_hints: LifetimeElisionHints::Never,
            param_names_for_lifetime_elision_hints: false,
            hide_inferred_type_hints: false,
            hide_named_constructor_hints: false,
            hide_closure_initialization_hints: false,
            hide_closure_parameter_hints: false,
            range_exclusive_hints: false,
            closure_style: ClosureStyle::ImplFn,
            max_length: Some(200),
            closing_brace_hints_min_lines: None,
            fields_to_resolve: InlayFieldsToResolve::empty(),
            ra_fixture: RaFixtureConfig::default(),
        };
        let hints = analysis
            .inlay_hints(&config, file_id, range)
            .map_err(|error| anyhow!("Rust inlay hints were cancelled: {error:?}"))?;
        Ok(Value::Array(
            hints
                .into_iter()
                .take(MAX_ENGINE_RESULTS + 1)
                .map(|hint| {
                    let offset = match hint.position {
                        InlayHintPosition::Before => hint.range.start(),
                        InlayHintPosition::After => hint.range.end(),
                    };
                    json!({
                        "position": offset_to_lsp(&source, text_size_to_usize(offset)),
                        "label": hint.label.to_string(),
                        "kind": inlay_kind_number(hint.kind),
                        "paddingLeft": hint.pad_left,
                        "paddingRight": hint.pad_right
                    })
                })
                .collect(),
        ))
    }

    fn code_actions(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let file_id = self.file_id_for_params(params)?;
        let source = self.file_text(analysis, file_id)?;
        let range = lsp_range_to_text(&source, params.get("range").unwrap_or(&Value::Null))?;
        let diagnostics_config = DiagnosticsConfig::test_sample();
        let assist_config = AssistConfig {
            snippet_cap: None,
            allowed: None,
            insert_use: InsertUseConfig {
                granularity: ImportGranularity::Item,
                enforce_granularity: false,
                prefix_kind: PrefixKind::Plain,
                group: false,
                skip_glob_imports: false,
            },
            prefer_no_std: false,
            prefer_prelude: true,
            prefer_absolute: false,
            assist_emit_must_use: true,
            term_search_fuel: 400,
            code_action_grouping: false,
            expr_fill_default: Default::default(),
            prefer_self_ty: false,
            show_rename_conflicts: true,
        };
        let assists = analysis
            .assists_with_fixes(
                &assist_config,
                &diagnostics_config,
                AssistResolveStrategy::All,
                FileRange { file_id, range },
            )
            .map_err(|error| anyhow!("Rust code action search was cancelled: {error:?}"))?;
        let mut actions = Vec::new();
        for assist in assists.into_iter().take(MAX_ENGINE_RESULTS) {
            let Some(source_change) = assist.source_change.as_ref() else {
                continue;
            };
            // Skip actions that need file creation, deletion, or renaming: the
            // edit preview interface currently accepts only source text edits.
            if !source_change.file_system_edits.is_empty() {
                continue;
            }
            let edit = self.source_change_to_workspace_edit(analysis, source_change)?;
            let mut action = json!({
                "title": assist.label.to_string(),
                "kind": assist.id.1.name(),
                "edit": edit
            });
            if let Some(group) = assist.group {
                action["group"] = json!(group.0);
            }
            actions.push(action);
        }
        Ok(Value::Array(actions))
    }

    fn code_lenses(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let file_id = self.file_id_for_params(params)?;
        let source = self.file_text(analysis, file_id)?;
        let runnables = analysis
            .runnables(file_id)
            .map_err(|error| anyhow!("Rust runnable discovery was cancelled: {error:?}"))?;
        let results = runnables
            .into_iter()
            .take(MAX_ENGINE_RESULTS)
            .map(|runnable| {
                json!({
                    "range": text_range_to_lsp(&source, runnable.nav.focus_or_full_range()),
                    "command": {
                        "title": runnable.title(),
                        "command": "aimer.rust.run",
                        "arguments": [runnable.label(None)]
                    }
                })
            })
            .collect();
        Ok(Value::Array(results))
    }

    fn expand_macro(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let expanded = analysis
            .expand_macro(position)
            .map_err(|error| anyhow!("Rust macro expansion was cancelled: {error:?}"))?;
        Ok(expanded.map_or(
            Value::Null,
            |expanded| json!({ "name": expanded.name, "expansion": expanded.expansion }),
        ))
    }

    fn prepare_call_hierarchy(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<Value> {
        let position = self.position(analysis, params)?;
        let candidates = analysis
            .call_hierarchy(
                position,
                &CallHierarchyConfig {
                    exclude_tests: false,
                    ra_fixture: RaFixtureConfig::default(),
                },
            )
            .map_err(|error| anyhow!("Rust call hierarchy was cancelled: {error:?}"))?;
        let mut items = Vec::new();
        for target in candidates
            .map(|candidate| candidate.info)
            .unwrap_or_default()
        {
            items.push(self.call_hierarchy_item(analysis, &target)?);
        }
        Ok(Value::Array(items))
    }

    fn call_hierarchy_calls(
        &self,
        analysis: &Analysis,
        params: &Value,
        incoming: bool,
    ) -> anyhow::Result<Value> {
        let item = params
            .get("item")
            .ok_or_else(|| anyhow!("call hierarchy request omitted item"))?;
        let uri = item
            .pointer("/data/uri")
            .or_else(|| item.get("uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("call hierarchy item omitted URI"))?;
        let path = uri_to_path(uri)?;
        let file_id = self.file_id_for_path(&path)?;
        let source = self.file_text(analysis, file_id)?;
        let position_value = item
            .pointer("/selectionRange/start")
            .or_else(|| item.pointer("/range/start"))
            .unwrap_or(&Value::Null);
        let offset = source_offset(&source, position_value)?;
        let position = FilePosition {
            file_id,
            offset: TextSize::from(offset as u32),
        };
        let config = CallHierarchyConfig {
            exclude_tests: false,
            ra_fixture: RaFixtureConfig::default(),
        };
        let calls = if incoming {
            analysis.incoming_calls(&config, position)
        } else {
            analysis.outgoing_calls(&config, position)
        }
        .map_err(|error| anyhow!("Rust call hierarchy was cancelled: {error:?}"))?
        .unwrap_or_default();
        let mut result = Vec::new();
        for call in calls.into_iter().take(MAX_ENGINE_RESULTS) {
            let target = self.call_hierarchy_item(analysis, &call.target)?;
            let ranges = call
                .ranges
                .into_iter()
                .map(|range| {
                    let source = self.file_text(analysis, range.file_id)?;
                    Ok(text_range_to_lsp(&source, range.range))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            if incoming {
                result.push(json!({ "from": target, "fromRanges": ranges }));
            } else {
                result.push(json!({ "to": target, "fromRanges": ranges }));
            }
        }
        Ok(Value::Array(result))
    }

    fn call_hierarchy_item(
        &self,
        analysis: &Analysis,
        target: &NavigationTarget,
    ) -> anyhow::Result<Value> {
        let (uri, range) = self.navigation_location(analysis, target)?;
        let (_, selection_range) = self.navigation_selection(analysis, target)?;
        let mut item = json!({
            "name": target.name.to_string(),
            "kind": target.kind.map(symbol_kind_number).unwrap_or(12),
            "uri": uri,
            "range": range,
            "selectionRange": selection_range,
            "data": { "uri": uri }
        });
        if let Some(detail) = &target.description {
            item["detail"] = json!(detail);
        }
        Ok(item)
    }

    fn source_change_to_workspace_edit(
        &self,
        analysis: &Analysis,
        change: &ra_ap_ide::SourceChange,
    ) -> anyhow::Result<Value> {
        if !change.file_system_edits.is_empty() {
            bail!("Rust code action includes a file operation that Aimer cannot preview");
        }
        let mut changes = serde_json::Map::new();
        for (file_id, (edit, _snippet)) in &change.source_file_edits {
            let uri = self.file_uri(*file_id)?;
            let source = self.file_text(analysis, *file_id)?;
            let edits = edit
                .iter()
                .map(|indel| {
                    json!({
                        "range": text_range_to_lsp(&source, indel.delete),
                        "newText": indel.insert
                    })
                })
                .collect::<Vec<_>>();
            changes.insert(uri, Value::Array(edits));
        }
        Ok(json!({ "changes": changes }))
    }

    fn position(&self, analysis: &Analysis, params: &Value) -> anyhow::Result<FilePosition> {
        let file_id = self.file_id_for_params(params)?;
        let source = self.file_text(analysis, file_id)?;
        let position = params.get("position").unwrap_or(&Value::Null);
        let offset = source_offset(&source, position)?;
        Ok(FilePosition {
            file_id,
            offset: TextSize::from(offset as u32),
        })
    }

    fn file_id_for_params(&self, params: &Value) -> anyhow::Result<FileId> {
        let uri = params
            .pointer("/textDocument/uri")
            .or_else(|| params.pointer("/item/uri"))
            .or_else(|| params.pointer("/item/data/uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("request omitted textDocument URI"))?;
        self.file_id_for_path(&uri_to_path(uri)?)
    }

    fn file_id_for_path(&self, path: &Path) -> anyhow::Result<FileId> {
        let path = self.resolve_path(path)?;
        let vfs_path = VfsPath::new_real_path(path.to_string_lossy().into_owned());
        match self.vfs.file_id(&vfs_path) {
            Some((file_id, FileExcluded::No)) => Ok(file_id),
            Some((_, FileExcluded::Yes)) => bail!(
                "Rust source '{}' is excluded from the workspace",
                path.display()
            ),
            None => bail!(
                "Rust source '{}' is not present in the loaded workspace",
                path.display()
            ),
        }
    }

    fn file_text(&self, analysis: &Analysis, file_id: FileId) -> anyhow::Result<String> {
        analysis
            .file_text(file_id)
            .map(|text| text.to_string())
            .map_err(|error| {
                anyhow!("reading Rust source from the analysis database was cancelled: {error:?}")
            })
    }

    fn file_uri(&self, file_id: FileId) -> anyhow::Result<String> {
        let path = self
            .vfs
            .file_path(file_id)
            .as_path()
            .ok_or_else(|| anyhow!("analysis target is not a filesystem path"))?;
        reqwest::Url::from_file_path(path.as_os_str())
            .map(|uri| uri.to_string())
            .map_err(|_| anyhow!("cannot convert analysis path '{}' to a file URI", path))
    }

    fn relative_file(&self, file_id: FileId) -> String {
        let path = self.vfs.file_path(file_id).to_string();
        Path::new(&path)
            .strip_prefix(&self.root)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or(path)
    }

    fn navigation_location(
        &self,
        analysis: &Analysis,
        target: &NavigationTarget,
    ) -> anyhow::Result<(String, Value)> {
        let uri = self.file_uri(target.file_id)?;
        let source = self.file_text(analysis, target.file_id)?;
        Ok((uri, text_range_to_lsp(&source, target.full_range)))
    }

    fn navigation_selection(
        &self,
        analysis: &Analysis,
        target: &NavigationTarget,
    ) -> anyhow::Result<(String, Value)> {
        let uri = self.file_uri(target.file_id)?;
        let source = self.file_text(analysis, target.file_id)?;
        Ok((
            uri,
            text_range_to_lsp(&source, target.focus_or_full_range()),
        ))
    }

    fn resolve_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("resolving '{}'", candidate.display()))?;
        if !canonical.starts_with(&self.root) {
            bail!("file path must stay inside the selected project");
        }
        Ok(canonical)
    }
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

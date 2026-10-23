use super::*;

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
        // Build scripts run through `cargo check`, which holds the lock on the
        // target directory while it works. In the shared `target/debug` that
        // stalls every `aimer run` build started meanwhile ("Blocking waiting
        // for file lock"), so the analyzer works in its own subdirectory.
        cargo_config.target_dir_config = TargetDirectoryConfig::UseSubdirectory;
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
        let engine_truncated = symbols.len() > MAX_ENGINE_RESULTS;
        let results = symbols
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
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut result = json!({ "results": results, "complete": !engine_truncated });
        if engine_truncated {
            result["reason"] = json!("engine_result_limit");
            result["engine_limit"] = json!(MAX_ENGINE_RESULTS);
        }
        Ok(result)
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
        let result_limit = if operation == "implementation" {
            MAX_IMPLEMENTATION_RESULTS
        } else {
            MAX_ENGINE_RESULTS
        };
        let engine_truncated = targets.len() > result_limit;
        let mut locations = Vec::new();
        for target in targets.into_iter().take(result_limit) {
            let (uri, range) = self.navigation_location(analysis, &target)?;
            let (_, selection_range) = self.navigation_selection(analysis, &target)?;
            locations.push(json!({
                "targetUri": uri,
                "targetRange": range,
                "targetSelectionRange": selection_range
            }));
        }
        if operation == "implementation" {
            let mut result = json!({
                "results": locations,
                "complete": !engine_truncated
            });
            if engine_truncated {
                result["reason"] = json!("engine_result_limit");
                result["engine_limit"] = json!(result_limit);
            }
            Ok(result)
        } else {
            Ok(Value::Array(locations))
        }
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
}

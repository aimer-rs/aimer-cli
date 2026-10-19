use super::implementation_results::implementation_page;
use super::*;

#[tool_router(router = ide_tool_router, vis = "pub(super)")]
impl AimerMcpServer {
    #[tool(
        output_schema = mcp_result_schema(),
        name = "search_symbols",
        description = "Fuzzy-search Rust workspace symbols. Rust kind filters include trait, impl, and macro; unknown kinds are rejected. Results include crate, module_path, and is_reexport metadata."
    )]
    async fn rust_search_symbols(
        &self,
        Parameters(arguments): Parameters<RustSearchSymbolsArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            validate_rust_limit(arguments.limit)?;
            anyhow::ensure!(
                !arguments.query.trim().is_empty(),
                "query must not be empty"
            );
            anyhow::ensure!(
                arguments.query.len() <= 512,
                "query must be at most 512 bytes"
            );
            let query = arguments.query.trim();
            let response = self
                .rust_analyzer
                .request("workspace/symbol", json!({ "query": query }), &progress)
                .await?;
            let symbols = response
                .as_array()
                .or_else(|| response.get("results").and_then(Value::as_array))
                .cloned()
                .unwrap_or_default();
            let engine_complete = response
                .get("complete")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let kind_filter = arguments
                .kind
                .as_deref()
                .map(normalize_rust_symbol_kind)
                .transpose()?;
            let root = self.rust_analyzer.project_root();
            let mut matching = symbols
                .into_iter()
                .filter_map(|symbol| {
                    let metadata = symbol_metadata(&root, &symbol);
                    let name = symbol
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let raw_kind = rust_symbol_kind_name(
                        symbol.get("kind").and_then(Value::as_u64).unwrap_or(0),
                    );
                    let kind = if name.trim_start().starts_with("impl ") {
                        "impl"
                    } else if metadata.is_macro {
                        "macro"
                    } else {
                        match raw_kind {
                            "interface" => "trait",
                            "class" => "struct",
                            "enum_member" => "variant",
                            "value" => "variable",
                            other => other,
                        }
                    };
                    kind_filter
                        .as_ref()
                        .is_none_or(|filter| filter == kind)
                        .then_some((symbol, metadata, kind.to_owned()))
                })
                .collect::<Vec<_>>();
            // rust-analyzer can return equally relevant symbols in different
            // orders across fresh sessions. Use a canonical key while keeping
            // exact name/path matches ahead of fuzzy matches.
            matching.sort_by_cached_key(|(symbol, _, _)| rust_symbol_sort_key(symbol, query));
            let limit = arguments.limit.unwrap_or(50).min(200);
            let truncated = matching.len() > limit;
            let results = matching
                .into_iter()
                .take(limit)
                .map(|(symbol, metadata, kind)| {
                    let match_quality = if rust_symbol_is_exact_match(&symbol, query) {
                        "exact"
                    } else {
                        "fuzzy"
                    };
                    let mut result = normalize_result(&root, &symbol);
                    if let Some(result) = result.as_object_mut() {
                        result.insert("match_quality".to_owned(), json!(match_quality));
                        result.insert("kind".to_owned(), json!(kind));
                        result.insert("crate".to_owned(), json!(metadata.package));
                        result.insert("module_path".to_owned(), json!(metadata.module_path));
                        result.insert("is_reexport".to_owned(), json!(metadata.is_reexport));
                    }
                    result
                })
                .collect::<Vec<_>>();
            let no_results = results.is_empty();
            let mut result = json!({ "results": results, "complete": engine_complete });
            if truncated {
                result["truncated"] = Value::Bool(true);
            }
            if no_results && let Some(kind) = kind_filter {
                result["hint"] = json!(format!(
                    "No symbols matched kind '{kind}'. Omit kind to inspect the available matches."
                ));
            }
            if !engine_complete {
                result["reason"] = response
                    .get("reason")
                    .cloned()
                    .unwrap_or_else(|| json!("engine_returned_partial_results"));
                if let Some(engine_limit) = response.get("engine_limit") {
                    result["engine_limit"] = engine_limit.clone();
                }
            }
            Ok(result)
        }
        .await;
        encode("search_symbols", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "search_text",
        description = "Search project text, comments, and string literals. Set regex to interpret query as a Rust regular expression. Respects Git ignore rules when available and returns bounded matches."
    )]
    async fn rust_search_text(
        &self,
        Parameters(arguments): Parameters<RustTextSearchArguments>,
    ) -> CallToolResult {
        let project_root = self.rust_analyzer.project_root();
        let result = search_text(
            &project_root,
            &arguments.query,
            arguments.file_glob.as_deref(),
            arguments.case_sensitive,
            arguments.regex,
            arguments.limit.unwrap_or(50),
        )
        .await;
        encode("search_text", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "inspect_file",
        description = "Return structured file ranges and bounded literal or regex matches. Set format to text for numbered source lines."
    )]
    async fn rust_inspect_file(
        &self,
        Parameters(arguments): Parameters<RustInspectFileArguments>,
    ) -> CallToolResult {
        let ranges = arguments
            .ranges
            .into_iter()
            .map(|range| InspectFileRange {
                start_line: range.start_line,
                end_line: range.end_line,
            })
            .collect::<Vec<_>>();
        let searches = arguments
            .searches
            .into_iter()
            .map(|search| InspectFilePattern {
                pattern: search.pattern,
                regex: search.regex,
                case_sensitive: search.case_sensitive.unwrap_or(true),
                context_before: search.context_before.unwrap_or(0),
                context_after: search.context_after.unwrap_or(0),
            })
            .collect::<Vec<_>>();
        let result = inspect_file(
            &self.rust_analyzer.project_root(),
            &arguments.file,
            &ranges,
            &searches,
        )
        .await;
        encode_inspect_file(result, arguments.format.as_deref())
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "get_document_symbols",
        description = "Return a compact Rust symbol outline. Defaults to depth 2 and omits local variables and fields. Filter with kinds and name_filter; matching ancestors are kept for context."
    )]
    async fn rust_get_document_symbols(
        &self,
        Parameters(arguments): Parameters<RustDocumentSymbolsArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let (uri, _) = self
                .rust_analyzer
                .sync_document(Path::new(&arguments.file), &progress)
                .await?;
            let symbols = self
                .rust_analyzer
                .request(
                    "textDocument/documentSymbol",
                    json!({ "textDocument": { "uri": uri } }),
                    &progress,
                )
                .await?;
            Ok(compact_document_symbols(
                &symbols,
                arguments.max_depth.unwrap_or(2),
                &arguments.kinds,
                arguments.name_filter.as_deref(),
                arguments.include_variables,
                arguments.include_fields,
            )?)
        }
        .await;
        encode("get_document_symbols", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "completion",
        description = "Return Rust completion suggestions. Defaults to 50 items without documentation; request shortened documentation with include_documentation."
    )]
    async fn rust_completion(
        &self,
        Parameters(arguments): Parameters<RustCompletionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let limit = arguments.limit.unwrap_or(50);
            validate_rust_limit(Some(limit))?;
            let (uri, source) = self
                .rust_analyzer
                .sync_document(Path::new(&arguments.file), &progress)
                .await?;
            let position = lsp_position(&source, arguments.line, arguments.column)?;
            let response = self
                .rust_analyzer
                .request(
                    "textDocument/completion",
                    json!({ "textDocument": { "uri": uri }, "position": position }),
                    &progress,
                )
                .await?;
            Ok(normalize_completion(
                &self.rust_analyzer.project_root(),
                &response,
                limit,
                arguments.include_documentation,
            ))
        }
        .await;
        encode("completion", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "get_code_lenses",
        description = "Return bounded code lenses for a Rust file, resolving their command metadata without executing commands."
    )]
    async fn rust_get_code_lenses(
        &self,
        Parameters(arguments): Parameters<RustFileArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let (uri, _) = self
                .rust_analyzer
                .sync_document(Path::new(&arguments.file), &progress)
                .await?;
            let response = self
                .rust_analyzer
                .request(
                    "textDocument/codeLens",
                    json!({ "textDocument": { "uri": uri } }),
                    &progress,
                )
                .await?;
            let lenses = response.as_array().cloned().unwrap_or_default();
            let truncated = lenses.len() > 200;
            let mut results = Vec::new();
            for mut lens in lenses.into_iter().take(200) {
                if lens.get("command").is_none() && lens.get("data").is_some() {
                    if let Ok(resolved) = self
                        .rust_analyzer
                        .request("codeLens/resolve", lens.clone(), &progress)
                        .await
                    {
                        lens = resolved;
                    }
                }
                results.push(normalize_result(&self.rust_analyzer.project_root(), &lens));
            }
            let mut result = json!({ "results": results });
            if truncated {
                result["truncated"] = Value::Bool(true);
            }
            Ok(result)
        }
        .await;
        encode("get_code_lenses", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "find_definition",
        description = "Find the definition of the Rust symbol at a project-relative file position."
    )]
    async fn rust_find_definition(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        self.rust_position_request("textDocument/definition", arguments, &progress)
            .await
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "find_declaration",
        description = "Find the declaration of the Rust symbol at a project-relative file position."
    )]
    async fn rust_find_declaration(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        self.rust_position_request("textDocument/declaration", arguments, &progress)
            .await
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "find_references",
        description = "Find Rust symbol references at a project-relative file position."
    )]
    async fn rust_find_references(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let include_declaration = true;
        let result = self
            .rust_position_request_value(
                "textDocument/references",
                arguments,
                json!({ "context": { "includeDeclaration": include_declaration } }),
                &progress,
            )
            .await;
        encode("find_references", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "find_implementations",
        description = "Find sorted, labeled Rust implementations with pagination. Provide file, line, and column or a fully qualified symbol_path such as `aimer_widget::Drawable`. Test implementations are excluded by default; file_glob and include_tests can narrow or expand results."
    )]
    async fn rust_find_implementations(
        &self,
        Parameters(arguments): Parameters<RustImplementationsArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let (file, line, column, trait_name) =
                if let Some(symbol_path) = arguments.symbol_path.as_deref() {
                    anyhow::ensure!(
                        arguments.file.is_none()
                            && arguments.line.is_none()
                            && arguments.column.is_none(),
                        "use symbol_path or file/line/column, not both"
                    );
                    resolve_symbol_position(&self.rust_analyzer, symbol_path, &progress).await?
                } else {
                    let (Some(file), Some(line), Some(column)) =
                        (arguments.file, arguments.line, arguments.column)
                    else {
                        anyhow::bail!("provide symbol_path or all of file, line, and column");
                    };
                    anyhow::ensure!(!file.trim().is_empty(), "file must not be empty");
                    let (_, source) = self
                        .rust_analyzer
                        .sync_document(Path::new(&file), &progress)
                        .await?;
                    let trait_name = symbol_name_at_position(&source, line, column);
                    (file, line, column, trait_name)
                };
            let (uri, source) = self
                .rust_analyzer
                .sync_document(Path::new(&file), &progress)
                .await?;
            let position = lsp_position(&source, line, column)?;
            let raw = self
                .rust_analyzer
                .request(
                    "textDocument/implementation",
                    json!({ "textDocument": { "uri": uri }, "position": position }),
                    &progress,
                )
                .await?;
            implementation_page(
                &self.rust_analyzer.project_root(),
                &raw,
                trait_name.as_deref(),
                arguments.limit.unwrap_or(50),
                arguments.offset.unwrap_or(0),
                arguments.file_glob.as_deref(),
                arguments.include_tests,
            )
        }
        .await;
        encode("find_implementations", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "type_definition",
        description = "Find the type definition of the Rust symbol at a project-relative file position."
    )]
    async fn rust_type_definition(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        self.rust_position_request("textDocument/typeDefinition", arguments, &progress)
            .await
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "hover",
        description = "Return the Rust type and documentation information at a project-relative file position."
    )]
    async fn rust_hover(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        self.rust_position_request("textDocument/hover", arguments, &progress)
            .await
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "document_highlights",
        description = "Find symbol occurrences highlighted within the current Rust document."
    )]
    async fn rust_document_highlights(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let (uri, source) = self
                .rust_analyzer
                .sync_document(Path::new(&arguments.file), &progress)
                .await?;
            let position = lsp_position(&source, arguments.line, arguments.column)?;
            let response = self
                .rust_analyzer
                .request(
                    "textDocument/documentHighlight",
                    json!({ "textDocument": { "uri": uri }, "position": position }),
                    &progress,
                )
                .await?;
            Ok(normalize_document_highlights(
                &self.rust_analyzer.project_root(),
                &response,
            ))
        }
        .await;
        encode("document_highlights", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "signature_help",
        description = "Return function signature and active parameter information at a Rust source position."
    )]
    async fn rust_signature_help(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        self.rust_position_request("textDocument/signatureHelp", arguments, &progress)
            .await
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "prepare_rename",
        description = "Check whether the Rust symbol at a source position can be renamed and return its target range and placeholder."
    )]
    async fn rust_prepare_rename(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let (uri, source) = self
                .rust_analyzer
                .sync_document(Path::new(&arguments.file), &progress)
                .await?;
            let position = lsp_position(&source, arguments.line, arguments.column)?;
            let response = self
                .rust_analyzer
                .request(
                    "textDocument/prepareRename",
                    json!({ "textDocument": { "uri": uri }, "position": position }),
                    &progress,
                )
                .await?;
            Ok(normalize_prepare_rename(
                &self.rust_analyzer.project_root(),
                &response,
            ))
        }
        .await;
        encode("prepare_rename", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "inlay_hints",
        description = "Return inferred type and parameter-name hints for a Rust source range."
    )]
    async fn rust_inlay_hints(
        &self,
        Parameters(arguments): Parameters<RustRangeArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let (uri, source) = self
                .rust_analyzer
                .sync_document(Path::new(&arguments.file), &progress)
                .await?;
            let range = lsp_range(
                &source,
                arguments.start_line,
                arguments.start_column,
                arguments.end_line,
                arguments.end_column,
            )?;
            let response = self
                .rust_analyzer
                .request(
                    "textDocument/inlayHint",
                    json!({ "textDocument": { "uri": uri }, "range": range }),
                    &progress,
                )
                .await?;
            Ok(normalize_inlay_hints(
                &self.rust_analyzer.project_root(),
                &response,
            ))
        }
        .await;
        encode("inlay_hints", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "get_diagnostics",
        description = "Return rust-analyzer diagnostics for files in the selected project; external Cargo registry diagnostics are omitted. Pass file to narrow the result."
    )]
    async fn rust_get_diagnostics(
        &self,
        Parameters(arguments): Parameters<RustDiagnosticsArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = self
            .rust_analyzer
            .diagnostics(arguments.file.as_deref().map(Path::new), &progress)
            .await;
        encode("get_diagnostics", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "start_lsp",
        description = "Load the embedded rust-analyzer engine for project_path (absolute or relative to the current Rust project). Selecting a different directory switches the workspace for later Rust tools. Omit project_path to use the current Rust project. Waits for workspace loading and reports progress and ready status."
    )]
    async fn rust_start_lsp(
        &self,
        Parameters(arguments): Parameters<RustStartLspArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        encode(
            "start_lsp",
            self.rust_analyzer
                .start(arguments.project_path.as_deref().map(Path::new), &progress)
                .await,
        )
    }
}

async fn resolve_symbol_position(
    rust_analyzer: &RustAnalyzerBridge,
    symbol_path: &str,
    progress: &McpProgressReporter,
) -> anyhow::Result<(String, u32, u32, Option<String>)> {
    let symbol_path = symbol_path.trim();
    anyhow::ensure!(!symbol_path.is_empty(), "symbol_path must not be empty");
    anyhow::ensure!(
        symbol_path.len() <= 512,
        "symbol_path must be at most 512 bytes"
    );
    let name = symbol_path
        .split("::")
        .filter(|part| !part.is_empty())
        .last()
        .ok_or_else(|| anyhow::anyhow!("symbol_path must contain a Rust symbol name"))?;
    let response = rust_analyzer
        .request("workspace/symbol", json!({ "query": name }), progress)
        .await?;
    let symbols = response
        .as_array()
        .or_else(|| response.get("results").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    let root = rust_analyzer.project_root();
    let normalized_path = symbol_path.replace('-', "_").to_ascii_lowercase();
    let mut exact_path = Vec::new();
    let mut exact_name = Vec::new();
    for symbol in symbols {
        if !symbol
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            continue;
        }
        let metadata = symbol_metadata(&root, &symbol);
        let module_path = metadata.module_path.as_deref().unwrap_or_default();
        let full_path = format!("{module_path}::{}", name)
            .replace('-', "_")
            .to_ascii_lowercase();
        let target_without_crate = normalized_path
            .strip_prefix("crate::")
            .unwrap_or(&normalized_path);
        if full_path == normalized_path
            || full_path.ends_with(&format!("::{normalized_path}"))
            || full_path.ends_with(&format!("::{target_without_crate}"))
        {
            exact_path.push((symbol.clone(), metadata.clone()));
        }
        exact_name.push((symbol, metadata));
    }
    let mut candidates = if exact_path.is_empty() {
        exact_name
    } else {
        exact_path
    };
    candidates.sort_by_key(|(_, metadata)| metadata.is_reexport);
    anyhow::ensure!(
        !candidates.is_empty(),
        "no workspace symbol matches '{symbol_path}'"
    );
    if candidates.len() > 1 {
        anyhow::bail!(
            "symbol_path '{symbol_path}' is ambiguous; use a more fully qualified module path"
        );
    }
    let (symbol, _) = candidates.pop().expect("one candidate was checked");
    let uri = symbol
        .pointer("/location/uri")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("workspace symbol has no source URI"))?;
    let path = reqwest::Url::parse(uri)
        .ok()
        .and_then(|uri| uri.to_file_path().ok())
        .ok_or_else(|| anyhow::anyhow!("workspace symbol has an invalid source URI"))?;
    let root = root.canonicalize()?;
    let path = path.canonicalize()?;
    let file = path
        .strip_prefix(&root)
        .map_err(|_| anyhow::anyhow!("workspace symbol is outside the selected project"))?
        .to_string_lossy()
        .replace('\\', "/");
    let line = symbol
        .pointer("/location/range/start/line")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32
        + 1;
    let column = symbol
        .pointer("/location/range/start/character")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32
        + 1;
    let trait_name = (rust_symbol_kind_name(
        symbol
            .get("kind")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    ) == "interface")
        .then(|| name.to_owned());
    Ok((file, line, column, trait_name))
}

fn symbol_name_at_position(source: &str, line: u32, column: u32) -> Option<String> {
    struct TraitFinder {
        line: usize,
        name: Option<String>,
    }
    impl<'ast> syn::visit::Visit<'ast> for TraitFinder {
        fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
            let start = syn::spanned::Spanned::span(item).start().line;
            let end = syn::spanned::Spanned::span(item).end().line.max(start);
            if start <= self.line && self.line <= end {
                self.name = Some(item.ident.to_string());
            }
            syn::visit::visit_item_trait(self, item);
        }
    }
    let _ = column;
    let syntax = syn::parse_file(source).ok()?;
    let mut finder = TraitFinder {
        line: line as usize,
        name: None,
    };
    syn::visit::Visit::visit_file(&mut finder, &syntax);
    finder.name
}

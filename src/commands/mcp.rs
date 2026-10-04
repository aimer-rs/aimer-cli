use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServiceExt, schemars, tool, tool_handler, tool_router, transport::stdio};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::mcp_cargo::{
    CargoRunArguments, ManifestEditArguments, ManifestReadArguments, prepare_manifest_edit,
    read_manifest, read_project_metadata, run_cargo,
};
use super::mcp_docs::{DocsRsClient, GetCrateDocsArguments, SearchCrateDocsArguments};
use super::mcp_ra::{
    McpProgressReporter, RustAnalyzerBridge, normalize_bounded, normalize_result, search_text,
};
use crate::resource_monitor::ResourceRecordStore;
use crate::session::{
    ControlClient, SessionDescriptor, SessionRegistry, SessionSnapshot, SessionStatus,
};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct LogsArguments {
    /// One of all, build, app, or system.
    #[serde(default)]
    stream: Option<String>,
    /// Return entries strictly after this sequence number.
    #[serde(default)]
    cursor: Option<u64>,
    /// Restrict results to one run ID.
    #[serde(default)]
    run_id: Option<u64>,
    /// Maximum number of entries to return.
    #[serde(default)]
    limit: Option<usize>,
    /// Include source file and line information when available.
    #[serde(default)]
    include_source_locations: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct ResourceIntervalArguments {
    /// Requested sampling interval in milliseconds (500–3000). Omit to restore
    /// the global Aimer configuration value, which defaults to 1000 ms.
    #[serde(default)]
    interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct ResourceCaptureArguments {
    /// Capture duration in seconds (0.5–300), and at least as long as interval.
    #[schemars(range(min = 0.5, max = 300.0))]
    duration: f64,
    /// Sampling interval in seconds (0.5–3.0).
    #[schemars(range(min = 0.5, max = 3.0))]
    interval: f64,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct ResourceRecordingsArguments {
    /// Return entries after this list offset.
    #[serde(default)]
    cursor: Option<u64>,
    /// Maximum recordings to return (1–100).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct ResourceRecordingArguments {
    /// Stable recording ID returned by aimer_list_resource_recordings.
    recording_id: String,
    /// Return samples starting at this zero-based offset.
    #[serde(default)]
    cursor: Option<u64>,
    /// Maximum samples to return (1–2000).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustSearchSymbolsArguments {
    /// Fuzzy symbol name or path. Exact case-insensitive name/path matches are ranked first.
    query: String,
    /// Optional Rust symbol kind, such as function, struct, enum, trait, or module.
    #[serde(default)]
    kind: Option<String>,
    /// Maximum results (1–200; defaults to 50).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustTextSearchArguments {
    /// Text or substring to find in project files.
    query: String,
    /// Optional simple glob to restrict paths, for example `src/**/*.rs`.
    #[serde(default)]
    file_glob: Option<String>,
    /// Whether matching should be case-sensitive.
    #[serde(default)]
    case_sensitive: bool,
    /// Maximum results (1–200; defaults to 50).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustFileArguments {
    /// Project-relative Rust source path.
    file: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustPositionArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based source line.
    line: u32,
    /// 1-based UTF-16 code-unit column, matching LSP positions.
    column: u32,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustDiagnosticsArguments {
    /// Optional project-relative Rust source path. Omit to return all known diagnostics.
    #[serde(default)]
    file: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RustStartLspArguments {
    /// Optional absolute path, or path relative to the currently selected Rust project.
    #[serde(default)]
    project_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustCallHierarchyArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based source line.
    line: u32,
    /// 1-based UTF-16 code-unit column, matching LSP positions.
    column: u32,
    /// `incoming` for callers or `outgoing` for callees.
    direction: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustRangeArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based range start line and UTF-16 code-unit column.
    start_line: u32,
    start_column: u32,
    /// 1-based range end line and UTF-16 code-unit column.
    end_line: u32,
    end_column: u32,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustFormatRangeArguments {
    /// 1-based range start line and UTF-16 code-unit column.
    start_line: u32,
    start_column: u32,
    /// 1-based range end line and UTF-16 code-unit column.
    end_line: u32,
    end_column: u32,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustFormatArguments {
    /// Project-relative Rust source path.
    file: String,
    /// Optional range to format. Omit to format the whole file.
    #[serde(default)]
    range: Option<RustFormatRangeArguments>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustRenameArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based source line and column of the symbol.
    line: u32,
    column: u32,
    /// New Rust identifier name.
    new_name: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustCodeActionPreviewArguments {
    /// Opaque action_id returned by aimer_rust_get_code_actions.
    action_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustApplyEditArguments {
    /// Opaque preview_id returned by a Rust code or manifest preview tool.
    preview_id: String,
}

#[derive(Clone)]
struct AimerMcpServer {
    client: Option<ControlClient>,
    docs: DocsRsClient,
    project_root: PathBuf,
    rust_analyzer: RustAnalyzerBridge,
    resource_capture_lock: Arc<tokio::sync::Mutex<()>>,
}

#[tool_router]
impl AimerMcpServer {
    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_status",
        description = "Return the current Aimer run status and app process identity."
    )]
    fn status(&self) -> CallToolResult {
        encode("aimer_status", self.request_live("status", Value::Null))
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_restart_project",
        description = "Restart the attached Aimer project and wait for the new app run to start."
    )]
    fn restart_project(&self) -> CallToolResult {
        encode(
            "aimer_restart_project",
            self.request_live("restart", Value::Null),
        )
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_stop_project",
        description = "Stop the attached Aimer project."
    )]
    fn stop_project(&self) -> CallToolResult {
        encode("aimer_stop_project", self.request_live("stop", Value::Null))
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_get_logs",
        description = "Read bounded Aimer build, app, or system logs with cursor pagination."
    )]
    fn get_logs(&self, Parameters(arguments): Parameters<LogsArguments>) -> CallToolResult {
        encode(
            "aimer_get_logs",
            self.request_live("logs", logs_payload(arguments)),
        )
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_clear_logs",
        description = "Clear retained Aimer logs and return the sequence boundary that was cleared."
    )]
    fn clear_logs(&self, Parameters(arguments): Parameters<LogsArguments>) -> CallToolResult {
        encode(
            "aimer_clear_logs",
            self.request_live("clear_logs", logs_payload(arguments)),
        )
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_get_resource_usage",
        description = "Return the latest automatic CPU, memory, PID, thread, and GPU resource sample for the attached running Aimer app."
    )]
    fn get_resource_usage(&self) -> CallToolResult {
        encode(
            "aimer_get_resource_usage",
            self.request_live("resource_usage", Value::Null),
        )
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_set_resource_monitor_interval",
        description = "Set the attached app's live resource sampling interval to 500–3000 ms. Omit interval_ms to restore the global Aimer configuration default (1000 ms)."
    )]
    async fn set_resource_monitor_interval(
        &self,
        Parameters(arguments): Parameters<ResourceIntervalArguments>,
    ) -> CallToolResult {
        let _capture_lock = self.resource_capture_lock.lock().await;
        let payload = arguments
            .interval_ms
            .map(|interval_ms| json!({ "interval_ms": interval_ms }))
            .unwrap_or_else(|| json!({}));
        encode(
            "aimer_set_resource_monitor_interval",
            self.request_live("set_resource_interval", payload),
        )
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_record_resource_usage",
        description = "Record a bounded window of the attached app's resource usage. Takes duration in seconds (0.5–300) and interval in seconds (0.5–3), temporarily changes the live sampling interval, restores it afterward, and returns samples plus the run recording ID."
    )]
    async fn record_resource_usage(
        &self,
        Parameters(arguments): Parameters<ResourceCaptureArguments>,
    ) -> CallToolResult {
        let result = self.capture_resource_usage(arguments).await;
        encode("aimer_record_resource_usage", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_list_resource_recordings",
        description = "List retained per-run resource recordings for the attached Aimer project, including completed runs."
    )]
    fn list_resource_recordings(
        &self,
        Parameters(arguments): Parameters<ResourceRecordingsArguments>,
    ) -> CallToolResult {
        let result = ResourceRecordStore::for_project(&self.project_root).and_then(|store| {
            let retention_days = crate::config::resource_monitor_config().retention_days;
            let page = store.list(retention_days, arguments.cursor, arguments.limit)?;
            serde_json::to_value(page).context("encoding resource recording list")
        });
        encode("aimer_list_resource_recordings", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_get_resource_recording",
        description = "Read cursor-paginated timestamped samples from a retained per-run resource recording."
    )]
    fn get_resource_recording(
        &self,
        Parameters(arguments): Parameters<ResourceRecordingArguments>,
    ) -> CallToolResult {
        let result = ResourceRecordStore::for_project(&self.project_root).and_then(|store| {
            let retention_days = crate::config::resource_monitor_config().retention_days;
            let page = store.read(
                &arguments.recording_id,
                arguments.cursor,
                arguments.limit,
                retention_days,
            )?;
            serde_json::to_value(page).context("encoding resource recording samples")
        });
        encode("aimer_get_resource_recording", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_get_crate_docs",
        description = "Read docs.rs documentation for a Rust crate or item. Trait lookups also return the trait signature, all method signatures, required/provided status, and each method's documentation. Uses the attached project's Cargo.lock version by default and caches crate documentation for offline use."
    )]
    async fn get_crate_docs(
        &self,
        Parameters(arguments): Parameters<GetCrateDocsArguments>,
    ) -> CallToolResult {
        let result = self
            .docs
            .get_document(arguments)
            .await
            .and_then(|document| serde_json::to_value(document).context("encoding docs.rs result"));
        encode("aimer_get_crate_docs", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_search_crate_docs",
        description = "Search public Rust item names, paths, and documentation in a docs.rs crate. Uses the attached project's Cargo.lock version by default, returns bounded results with item paths and summaries, and shares the offline documentation cache with aimer_get_crate_docs."
    )]
    async fn search_crate_docs(
        &self,
        Parameters(arguments): Parameters<SearchCrateDocsArguments>,
    ) -> CallToolResult {
        let result = self
            .docs
            .search_documents(arguments)
            .await
            .and_then(|results| {
                serde_json::to_value(results).context("encoding docs.rs search results")
            });
        encode("aimer_search_crate_docs", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_search_symbols",
        description = "Fuzzy-search Rust workspace symbols. Case-insensitive exact name or path matches come first, followed by fuzzy matches; each result includes match_quality. Returns bounded JSON locations."
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
            let symbols = response.as_array().cloned().unwrap_or_default();
            let kind_filter = arguments
                .kind
                .as_deref()
                .map(|kind| kind.to_ascii_lowercase());
            let mut matching = symbols
                .into_iter()
                .filter(|symbol| {
                    kind_filter.as_ref().is_none_or(|filter| {
                        rust_symbol_kind_name(
                            symbol.get("kind").and_then(Value::as_u64).unwrap_or(0),
                        )
                        .replace('_', "")
                            == filter.replace('_', "")
                    })
                })
                .collect::<Vec<_>>();
            // rust-analyzer can return equally relevant symbols in different
            // orders across fresh sessions. Use a canonical key while keeping
            // exact name/path matches ahead of fuzzy matches.
            sort_rust_symbols(&mut matching, query);
            let limit = arguments.limit.unwrap_or(50).min(200);
            let truncated = matching.len() > limit;
            let results = matching
                .into_iter()
                .take(limit)
                .map(|symbol| {
                    let match_quality = if rust_symbol_is_exact_match(&symbol, query) {
                        "exact"
                    } else {
                        "fuzzy"
                    };
                    let mut result = normalize_result(&self.rust_analyzer.project_root(), &symbol);
                    if let Some(result) = result.as_object_mut() {
                        result.insert("match_quality".to_owned(), json!(match_quality));
                    }
                    result
                })
                .collect::<Vec<_>>();
            Ok(json!({ "results": results, "truncated": truncated }))
        }
        .await;
        encode("aimer_rust_search_symbols", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_search_text",
        description = "Search project text, comments, and string literals; respects Git ignore rules when available and returns bounded JSON matches."
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
            arguments.limit.unwrap_or(50),
        )
        .await;
        encode("aimer_rust_search_text", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_get_document_symbols",
        description = "Return the nested Rust symbol outline for a project file."
    )]
    async fn rust_get_document_symbols(
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
            let symbols = self
                .rust_analyzer
                .request(
                    "textDocument/documentSymbol",
                    json!({ "textDocument": { "uri": uri } }),
                    &progress,
                )
                .await?;
            Ok(normalize_bounded(
                &self.rust_analyzer.project_root(),
                &symbols,
            ))
        }
        .await;
        encode("aimer_rust_get_document_symbols", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_completion",
        description = "Return bounded Rust completion suggestions at a project-relative source position."
    )]
    async fn rust_completion(
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
                    "textDocument/completion",
                    json!({ "textDocument": { "uri": uri }, "position": position }),
                    &progress,
                )
                .await?;
            Ok(normalize_completion(
                &self.rust_analyzer.project_root(),
                &response,
            ))
        }
        .await;
        encode("aimer_rust_completion", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_get_code_lenses",
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
            Ok(json!({ "results": results, "truncated": truncated }))
        }
        .await;
        encode("aimer_rust_get_code_lenses", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_find_definition",
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
        name = "aimer_rust_find_declaration",
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
        name = "aimer_rust_find_references",
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
        encode("aimer_rust_find_references", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_find_implementations",
        description = "Find Rust implementations for the symbol at a project-relative file position."
    )]
    async fn rust_find_implementations(
        &self,
        Parameters(arguments): Parameters<RustPositionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        self.rust_position_request("textDocument/implementation", arguments, &progress)
            .await
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_type_definition",
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
        name = "aimer_rust_hover",
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
        name = "aimer_rust_document_highlights",
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
        encode("aimer_rust_document_highlights", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_signature_help",
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
        name = "aimer_rust_prepare_rename",
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
        encode("aimer_rust_prepare_rename", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_inlay_hints",
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
        encode("aimer_rust_inlay_hints", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_get_diagnostics",
        description = "Return the latest rust-analyzer diagnostics as bounded structured JSON."
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
        encode("aimer_rust_get_diagnostics", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_start_lsp",
        description = "Start or reuse rust-analyzer for project_path (absolute or relative to the current Rust project). Selecting a different directory switches the workspace for later Rust tools. Omit project_path to use the current Rust project. Waits for indexing and reports progress and ready status."
    )]
    async fn rust_start_lsp(
        &self,
        Parameters(arguments): Parameters<RustStartLspArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        encode(
            "aimer_rust_start_lsp",
            self.rust_analyzer
                .start(arguments.project_path.as_deref().map(Path::new), &progress)
                .await,
        )
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_get_manifest",
        description = "Read a Cargo.toml as structured JSON. Omit manifest_path to read the selected Rust project's Cargo.toml."
    )]
    async fn rust_get_manifest(
        &self,
        Parameters(arguments): Parameters<ManifestReadArguments>,
    ) -> CallToolResult {
        let result = read_manifest(&self.rust_analyzer.project_root(), arguments).await;
        encode("aimer_rust_get_manifest", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_get_project_metadata",
        description = "Return bounded structured Cargo workspace, package, target, feature, dependency, and resolution metadata. Uses --locked and the configured Cargo network policy; metadata is unavailable if Cargo cannot resolve the existing lockfile."
    )]
    async fn rust_get_project_metadata(&self) -> CallToolResult {
        let result = read_project_metadata(
            &self.rust_analyzer.project_root(),
            self.rust_analyzer.allow_network(),
        )
        .await;
        encode("aimer_rust_get_project_metadata", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_preview_manifest_edit",
        description = "Preview ordered set/remove operations on a Cargo.toml key path. The candidate is parsed and checked with Cargo metadata in a temporary workspace; use aimer_rust_apply_edit to apply an applicable preview."
    )]
    async fn rust_preview_manifest_edit(
        &self,
        Parameters(arguments): Parameters<ManifestEditArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let root = self.rust_analyzer.project_root();
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let root = root
                .canonicalize()
                .context("resolving selected Rust project root")?;
            let candidate = prepare_manifest_edit(
                &root,
                self.rust_analyzer.allow_network(),
                arguments,
                Some(&progress),
            )
            .await?;
            let relative_manifest = candidate
                .manifest
                .strip_prefix(&root)
                .context("manifest is outside the selected Rust project")?;
            self.rust_analyzer
                .prepare_manifest_content_preview(
                    relative_manifest,
                    &candidate.before,
                    &candidate.after,
                    candidate.validation,
                    candidate.applicable,
                )
                .await
        }
        .await;
        encode("aimer_rust_preview_manifest_edit", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_cargo_check",
        description = "Run cargo check for the selected workspace or package. Returns Cargo's actual exit code, bounded output, and the number of rustc warnings filtered from the output. Cargo runs offline unless the existing rust-analyzer network setting opts in."
    )]
    async fn rust_cargo_check(
        &self,
        Parameters(arguments): Parameters<CargoRunArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = run_cargo(
            &self.rust_analyzer.project_root(),
            self.rust_analyzer.allow_network(),
            false,
            arguments,
            Some(&progress),
        )
        .await;
        encode("aimer_rust_cargo_check", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_cargo_test",
        description = "Run cargo test for the selected workspace or package, optionally with a test name filter. Returns Cargo's actual exit code, bounded output, and the number of rustc warnings filtered from the output. Cargo runs offline unless the existing rust-analyzer network setting opts in."
    )]
    async fn rust_cargo_test(
        &self,
        Parameters(arguments): Parameters<CargoRunArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = run_cargo(
            &self.rust_analyzer.project_root(),
            self.rust_analyzer.allow_network(),
            true,
            arguments,
            Some(&progress),
        )
        .await;
        encode("aimer_rust_cargo_test", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_check",
        description = "Run rust-analyzer's Clippy flycheck with all warnings denied (-D warnings). Returns a completion status, diagnostic counts, and bounded diagnostics; rust-analyzer does not expose Cargo's exit code. Requires Clippy to be installed. Timeout and runner errors include a code and phase. Cargo runs offline unless user config opts in to network access."
    )]
    async fn rust_check(
        &self,
        Parameters(arguments): Parameters<RustDiagnosticsArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = self
            .rust_analyzer
            .run_check(arguments.file.as_deref().map(Path::new), &progress)
            .await;
        encode("aimer_rust_check", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_get_call_hierarchy",
        description = "Return callers or callees for the function at a project-relative file position."
    )]
    async fn rust_get_call_hierarchy(
        &self,
        Parameters(arguments): Parameters<RustCallHierarchyArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let (uri, source) = self
                .rust_analyzer
                .sync_document(Path::new(&arguments.file), &progress)
                .await?;
            let position = lsp_position(&source, arguments.line, arguments.column)?;
            let items = self
                .rust_analyzer
                .request(
                    "textDocument/prepareCallHierarchy",
                    json!({ "textDocument": { "uri": uri }, "position": position }),
                    &progress,
                )
                .await?;
            let item = items
                .as_array()
                .and_then(|items| items.first())
                .cloned()
                .unwrap_or(Value::Null);
            if item.is_null() {
                return Ok(json!({ "results": [] }));
            }
            let method = match arguments.direction.as_str() {
                "incoming" => "callHierarchy/incomingCalls",
                "outgoing" => "callHierarchy/outgoingCalls",
                _ => anyhow::bail!("direction must be 'incoming' or 'outgoing'"),
            };
            let results = self
                .rust_analyzer
                .request(method, json!({ "item": item }), &progress)
                .await?;
            Ok(normalize_bounded(
                &self.rust_analyzer.project_root(),
                &results,
            ))
        }
        .await;
        encode("aimer_rust_get_call_hierarchy", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_expand_macro",
        description = "Expand the Rust macro invocation at a project-relative file position."
    )]
    async fn rust_expand_macro(
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
            let expansion = self
                .rust_analyzer
                .request(
                    "rust-analyzer/expandMacro",
                    json!({ "textDocument": { "uri": uri }, "position": position }),
                    &progress,
                )
                .await?;
            let mut expansion = expansion;
            if let Some(object) = expansion.as_object_mut() {
                let text = object
                    .get("expansion")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(text) = text.filter(|text| text.len() > 64 * 1024) {
                    object.insert(
                        "expansion".to_owned(),
                        Value::String(text.chars().take(64 * 1024).collect()),
                    );
                    object.insert("truncated".to_owned(), Value::Bool(true));
                }
            }
            Ok(expansion)
        }
        .await;
        encode("aimer_rust_expand_macro", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_get_code_actions",
        description = "List code actions available for a project-relative source range."
    )]
    async fn rust_get_code_actions(
        &self,
        Parameters(arguments): Parameters<RustRangeArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let (_, source) = self
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
            self.rust_analyzer
                .get_code_actions(Path::new(&arguments.file), range, &progress)
                .await
        }
        .await;
        encode("aimer_rust_get_code_actions", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_rename_symbol",
        description = "Prepare a semantic symbol rename and return a preview. Possible unedited identifiers in macro_rules! bodies are listed separately and are never added automatically; apply the preview with aimer_rust_apply_edit."
    )]
    async fn rust_rename_symbol(
        &self,
        Parameters(arguments): Parameters<RustRenameArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = self
            .rust_analyzer
            .prepare_rename(
                Path::new(&arguments.file),
                arguments.line,
                arguments.column,
                &arguments.new_name,
                &progress,
            )
            .await;
        encode("aimer_rust_rename_symbol", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_preview_code_action",
        description = "Resolve a code action and return a preview; apply it with aimer_rust_apply_edit."
    )]
    async fn rust_preview_code_action(
        &self,
        Parameters(arguments): Parameters<RustCodeActionPreviewArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = self
            .rust_analyzer
            .prepare_code_action(&arguments.action_id, &progress)
            .await;
        encode("aimer_rust_preview_code_action", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_format",
        description = "Format a Rust file or range and return a preview; apply it with aimer_rust_apply_edit."
    )]
    async fn rust_format(
        &self,
        Parameters(arguments): Parameters<RustFormatArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = async {
            let range = if let Some(range) = arguments.range {
                let (_, source) = self
                    .rust_analyzer
                    .sync_document(Path::new(&arguments.file), &progress)
                    .await?;
                Some(lsp_range(
                    &source,
                    range.start_line,
                    range.start_column,
                    range.end_line,
                    range.end_column,
                )?)
            } else {
                None
            };
            self.rust_analyzer
                .prepare_format(Path::new(&arguments.file), range, &progress)
                .await
        }
        .await;
        encode("aimer_rust_format", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "aimer_rust_apply_edit",
        description = "Apply a previously reviewed Rust source or Cargo.toml preview if its source files are unchanged and manifest validation allows it."
    )]
    async fn rust_apply_edit(
        &self,
        Parameters(arguments): Parameters<RustApplyEditArguments>,
    ) -> CallToolResult {
        let result = self
            .rust_analyzer
            .apply_preview(&arguments.preview_id)
            .await;
        encode("aimer_rust_apply_edit", result)
    }
}

#[tool_handler(
    instructions = "Rust LSP tools wait for rust-analyzer to finish loading and indexing the selected workspace before returning results. Keep an LSP tool call pending while it reports indexing progress; do not interpret an unfinished call as an empty search result. Readiness timeouts and rust-analyzer health errors are returned explicitly."
)]
impl rmcp::ServerHandler for AimerMcpServer {}

impl AimerMcpServer {
    async fn rust_position_request(
        &self,
        method: &str,
        arguments: RustPositionArguments,
        progress: &McpProgressReporter,
    ) -> CallToolResult {
        encode(
            method,
            self.rust_position_request_value(method, arguments, Value::Null, progress)
                .await,
        )
    }

    async fn rust_position_request_value(
        &self,
        method: &str,
        arguments: RustPositionArguments,
        extra: Value,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let (uri, source) = self
            .rust_analyzer
            .sync_document(Path::new(&arguments.file), progress)
            .await?;
        let position = lsp_position(&source, arguments.line, arguments.column)?;
        let mut params = json!({
            "textDocument": { "uri": uri },
            "position": position
        });
        if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            params.extend(extra.clone());
        }
        let result = self.rust_analyzer.request(method, params, progress).await?;
        Ok(normalize_bounded(
            &self.rust_analyzer.project_root(),
            &result,
        ))
    }

    fn request_live(&self, operation: &str, payload: Value) -> anyhow::Result<Value> {
        self.client
            .as_ref()
            .ok_or_else(|| {
                anyhow!("no live Aimer session is attached; use --attach with a session ID for live controls")
            })?
            .request(operation, payload)
    }

    async fn capture_resource_usage(
        &self,
        arguments: ResourceCaptureArguments,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!(
            arguments.interval.is_finite() && (0.5..=3.0).contains(&arguments.interval),
            "interval must be between 0.5 and 3 seconds"
        );
        anyhow::ensure!(
            arguments.duration.is_finite()
                && arguments.duration >= arguments.interval
                && arguments.duration <= 300.0,
            "duration must be at least interval and no more than 300 seconds"
        );

        let _capture_lock = self.resource_capture_lock.lock().await;
        let snapshot: SessionSnapshot =
            serde_json::from_value(self.request_live("ping", Value::Null)?)
                .context("decoding live Aimer session status")?;
        anyhow::ensure!(
            matches!(
                snapshot.status,
                SessionStatus::Running | SessionStatus::Error
            ) && snapshot.app_process.is_some(),
            "the attached Aimer app is not running"
        );

        let interval_ms = (arguments.interval * 1_000.0).round() as u64;
        let client = self
            .client
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("no live Aimer session is attached"))?;
        let mut interval_restore =
            ResourceIntervalRestore::new(client, snapshot.resource_monitor.interval_ms);
        interval_restore.set_interval(interval_ms)?;

        let capture_started_unix_ms = now_unix_ms()?;
        let capture_started = tokio::time::Instant::now();
        tokio::time::sleep(Duration::from_secs_f64(arguments.duration)).await;
        let capture_finished_unix_ms = now_unix_ms()?;
        let captured_duration_seconds = capture_started.elapsed().as_secs_f64();
        let interval_restore_error = interval_restore
            .restore()
            .err()
            .map(|error| error.to_string());

        let retention_days = crate::config::resource_monitor_config().retention_days;
        let store = ResourceRecordStore::for_project(&self.project_root)?;
        let recording_id = format!("{}_{}", snapshot.session_id, snapshot.run_id);
        let metadata = store
            .list(retention_days, None, Some(100))?
            .recordings
            .into_iter()
            .find(|recording| recording.recording_id == recording_id)
            .ok_or_else(|| anyhow!("resource recording '{recording_id}' was not found"))?;
        let expected_samples = (arguments.duration / arguments.interval).ceil() as u64 + 8;
        let cursor = metadata.sample_count.saturating_sub(expected_samples);
        let page = store.read(&recording_id, Some(cursor), Some(2_000), retention_days)?;
        let samples: Vec<_> = page
            .samples
            .into_iter()
            .filter(|sample| {
                (capture_started_unix_ms..=capture_finished_unix_ms)
                    .contains(&sample.timestamp_unix_ms)
            })
            .collect();

        Ok(json!({
            "recording_id": recording_id,
            "duration_seconds": captured_duration_seconds,
            "interval_seconds": interval_ms as f64 / 1_000.0,
            "capture_started_unix_ms": capture_started_unix_ms,
            "capture_finished_unix_ms": capture_finished_unix_ms,
            "sample_count": samples.len(),
            "samples": samples,
            "interval_restored": interval_restore_error.is_none(),
            "interval_restore_error": interval_restore_error,
        }))
    }
}

struct ResourceIntervalRestore {
    client: ControlClient,
    interval_ms: u64,
    armed: bool,
}

impl ResourceIntervalRestore {
    fn new(client: ControlClient, interval_ms: u64) -> Self {
        Self {
            client,
            interval_ms,
            armed: true,
        }
    }

    fn set_interval(&self, interval_ms: u64) -> anyhow::Result<()> {
        self.client.request(
            "set_resource_interval",
            json!({ "interval_ms": interval_ms }),
        )?;
        Ok(())
    }

    fn restore(&mut self) -> anyhow::Result<()> {
        self.set_interval(self.interval_ms)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for ResourceIntervalRestore {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.set_interval(self.interval_ms) {
                tracing::debug!(error = %error, "could not restore resource sampling interval");
            }
        }
    }
}

fn now_unix_ms() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis() as u64)
}

fn encode(operation: &str, result: anyhow::Result<Value>) -> CallToolResult {
    match result {
        Ok(value @ Value::Object(_)) => CallToolResult::structured(value),
        Ok(value) => CallToolResult::structured(json!({ "result": value })),
        Err(error) => {
            let message = format!("{error:#}");
            let lower_message = message.to_ascii_lowercase();
            let (code, phase) = if operation == "aimer_rust_check"
                && (lower_message.contains("check_start_timeout")
                    || lower_message.contains("did not start flycheck"))
            {
                ("check_start_timeout", "start")
            } else if operation == "aimer_rust_check"
                && lower_message.contains("check_runner_unavailable")
            {
                ("check_runner_unavailable", "runner")
            } else if operation == "aimer_rust_check"
                && (lower_message.contains("check_timeout")
                    || lower_message.contains("flycheck timed out")
                    || lower_message.contains("timed out waiting for rust-analyzer"))
            {
                ("check_timeout", "execution")
            } else if lower_message.contains("no live aimer session is attached") {
                ("session_unavailable", "attachment")
            } else if lower_message.contains("did not report a ready workspace")
                || (lower_message.contains("workspace") && lower_message.contains("index"))
            {
                ("workspace_not_ready", "indexing")
            } else if lower_message.contains("starting rust-analyzer")
                || lower_message.contains("could not start rust-analyzer")
            {
                ("lsp_start_failed", "startup")
            } else if lower_message.contains("timed out") || lower_message.contains("timeout") {
                ("timeout", "execution")
            } else if lower_message.contains("must not be empty")
                || lower_message.contains("must be")
                || lower_message.contains("invalid")
            {
                ("invalid_argument", "validation")
            } else if lower_message.contains("rust-analyzer") {
                ("lsp_error", "lsp_request")
            } else {
                ("operation_failed", "execution")
            };
            CallToolResult::structured_error(json!({
                "error": {
                    "code": code,
                    "operation": operation,
                    "phase": phase,
                    "message": message
                }
            }))
        }
    }
}

fn mcp_result_schema() -> Arc<Map<String, Value>> {
    let mut schema = Map::new();
    schema.insert("type".to_owned(), json!("object"));
    schema.insert("additionalProperties".to_owned(), json!(true));
    Arc::new(schema)
}

fn validate_rust_limit(limit: Option<usize>) -> anyhow::Result<()> {
    if limit.is_some_and(|limit| !(1..=200).contains(&limit)) {
        anyhow::bail!("limit must be between 1 and 200");
    }
    Ok(())
}

fn normalize_completion(root: &Path, value: &Value) -> Value {
    let items = value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let (results, truncated) = normalize_lsp_items(root, items, completion_kind_name, true);
    json!({
        "results": results,
        "is_incomplete": value
            .get("isIncomplete")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "truncated": truncated
    })
}

fn normalize_document_highlights(root: &Path, value: &Value) -> Value {
    let items = value.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let (results, truncated) =
        normalize_lsp_items(root, items, document_highlight_kind_name, false);
    json!({ "results": results, "truncated": truncated })
}

fn normalize_prepare_rename(root: &Path, value: &Value) -> Value {
    if value.get("range").is_some() || value.get("defaultBehavior").is_some() {
        return normalize_result(root, value);
    }
    if value.get("start").is_some() && value.get("end").is_some() {
        return normalize_result(root, &json!({ "range": value }))
            .get("range")
            .cloned()
            .unwrap_or(Value::Null);
    }
    normalize_result(root, value)
}

fn normalize_inlay_hints(root: &Path, value: &Value) -> Value {
    let items = value.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let (results, truncated) = normalize_lsp_items(root, items, inlay_hint_kind_name, false);
    json!({ "results": results, "truncated": truncated })
}

fn normalize_lsp_items(
    root: &Path,
    items: &[Value],
    kind_name: fn(u64) -> &'static str,
    truncate_documentation: bool,
) -> (Vec<Value>, bool) {
    let truncated = items.len() > 200;
    let results = items
        .iter()
        .take(200)
        .map(|item| {
            let kind = item.get("kind").and_then(Value::as_u64);
            let mut normalized = normalize_result(root, item);
            if let (Some(kind), Some(object)) = (kind, normalized.as_object_mut()) {
                object.insert("kind".to_owned(), Value::String(kind_name(kind).to_owned()));
            }
            if truncate_documentation {
                truncate_completion_documentation(&mut normalized);
            }
            normalized
        })
        .collect();
    (results, truncated)
}

fn truncate_completion_documentation(item: &mut Value) {
    const MAX_DOCUMENTATION_CHARS: usize = 8_000;
    let truncated = if let Some(documentation) = item.get_mut("documentation") {
        match documentation {
            Value::String(text) => truncate_text(text, MAX_DOCUMENTATION_CHARS),
            Value::Object(object) => match object.get_mut("value") {
                Some(Value::String(text)) => truncate_text(text, MAX_DOCUMENTATION_CHARS),
                _ => false,
            },
            _ => false,
        }
    } else {
        false
    };
    if truncated {
        if let Some(object) = item.as_object_mut() {
            object.insert("documentation_truncated".to_owned(), Value::Bool(true));
        }
    }
}

fn truncate_text(text: &mut String, max_chars: usize) -> bool {
    if text.chars().count() <= max_chars {
        return false;
    }
    *text = text.chars().take(max_chars).collect();
    text.push('…');
    true
}

fn completion_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "text",
        2 => "method",
        3 => "function",
        4 => "constructor",
        5 => "field",
        6 => "variable",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        11 => "unit",
        12 => "value",
        13 => "enum",
        14 => "keyword",
        15 => "snippet",
        16 => "color",
        17 => "file",
        18 => "reference",
        19 => "folder",
        20 => "enum_member",
        21 => "constant",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "type_parameter",
        _ => "other",
    }
}

fn document_highlight_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "text",
        2 => "read",
        3 => "write",
        _ => "other",
    }
}

fn inlay_hint_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "type",
        2 => "parameter",
        _ => "other",
    }
}

fn rust_symbol_is_exact_match(symbol: &Value, query: &str) -> bool {
    let query = query.trim().trim_start_matches("::");
    let name = symbol
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.eq_ignore_ascii_case(query) {
        return true;
    }

    let container = symbol
        .get("containerName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if container.is_empty() || name.is_empty() {
        return false;
    }

    let full_path = format!("{container}::{name}");
    let query = query.to_ascii_lowercase();
    let full_path = full_path.to_ascii_lowercase();
    query == full_path || query.ends_with(&format!("::{full_path}"))
}

fn rust_symbol_sort_key(
    symbol: &Value,
    query: &str,
) -> (bool, String, String, String, u64, u64, u64, String) {
    let name = symbol
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let container = symbol
        .get("containerName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let location = symbol.get("location");
    let uri = location
        .and_then(|location| location.get("uri"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let start = location
        .and_then(|location| location.get("range"))
        .and_then(|range| range.get("start"));

    (
        !rust_symbol_is_exact_match(symbol, query),
        name.to_ascii_lowercase(),
        container.to_ascii_lowercase(),
        uri.to_owned(),
        start
            .and_then(|position| position.get("line"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        start
            .and_then(|position| position.get("character"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        symbol
            .get("kind")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        serde_json::to_string(symbol).unwrap_or_default(),
    )
}

fn sort_rust_symbols(symbols: &mut [Value], query: &str) {
    symbols.sort_by_cached_key(|symbol| rust_symbol_sort_key(symbol, query));
}

fn rust_symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        2 | 3 => "module",
        5 => "class",
        6 => "method",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        22 => "enum_member",
        23 => "struct",
        25 => "operator",
        26 => "type_parameter",
        _ => "other",
    }
}

fn lsp_position(source: &str, line: u32, column: u32) -> anyhow::Result<Value> {
    if line == 0 || column == 0 {
        anyhow::bail!("line and column are 1-based and must be greater than zero");
    }
    let line_text = source
        .lines()
        .nth((line - 1) as usize)
        .ok_or_else(|| anyhow::anyhow!("line is outside the source file"))?;
    let requested_units = (column - 1) as usize;
    let mut units = 0;
    for ch in line_text.chars() {
        if units == requested_units {
            break;
        }
        units += ch.len_utf16();
        if units > requested_units {
            anyhow::bail!("column splits a UTF-16 surrogate pair");
        }
    }
    if units != requested_units {
        anyhow::bail!("column is outside the source line");
    }
    Ok(json!({ "line": line - 1, "character": requested_units }))
}

fn lsp_range(
    source: &str,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
) -> anyhow::Result<Value> {
    let start = lsp_position(source, start_line, start_column)?;
    let end = lsp_position(source, end_line, end_column)?;
    let start_key = (start_line, start_column);
    let end_key = (end_line, end_column);
    if start_key > end_key {
        anyhow::bail!("range start must not be after range end");
    }
    Ok(json!({ "start": start, "end": end }))
}

fn logs_payload(arguments: LogsArguments) -> Value {
    let mut payload = Map::new();
    if let Some(stream) = arguments.stream.filter(|stream| stream != "all") {
        payload.insert("stream".to_string(), Value::String(stream));
    }
    if let Some(cursor) = arguments.cursor {
        payload.insert("cursor".to_string(), json!(cursor));
    }
    if let Some(run_id) = arguments.run_id {
        payload.insert("run_id".to_string(), json!(run_id));
    }
    if let Some(limit) = arguments.limit {
        payload.insert("limit".to_string(), json!(limit));
    }
    if let Some(include) = arguments.include_source_locations {
        payload.insert("include_source_locations".to_string(), json!(include));
    }
    Value::Object(payload)
}

/// Run the MCP stdio adapter for a selected Aimer session.
pub fn execute(project: Option<PathBuf>, attach: Option<String>) -> anyhow::Result<()> {
    if project.is_some() && attach.is_some() {
        anyhow::bail!("--project and --attach are mutually exclusive");
    }
    let (client, project_root) = select_context(project, attach)?;
    let docs = DocsRsClient::new(project_root.clone())?;
    let rust_analyzer =
        RustAnalyzerBridge::new(project_root.clone(), crate::config::rust_analyzer_config());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting MCP stdio runtime")?;
    runtime.block_on(async move {
        let server = AimerMcpServer {
            client,
            docs,
            project_root,
            rust_analyzer,
            resource_capture_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
        Ok::<(), anyhow::Error>(())
    })
}

/// List live session descriptors for the sessions command.
pub fn list_sessions(project: Option<PathBuf>, json_output: bool) -> anyhow::Result<()> {
    let project = project.map(resolve_project_path).transpose()?;
    let registry = SessionRegistry::default_location()?;
    let sessions = registry.live(project.as_deref())?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("No live Aimer run sessions.");
        return Ok(());
    }
    for session in sessions {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            session.session_id,
            session.status_name(),
            session.target,
            session
                .app_process
                .as_ref()
                .and_then(|process| process.pid)
                .map_or_else(|| "-".to_string(), |pid| pid.to_string()),
            session.project_root.display(),
        );
    }
    Ok(())
}

fn select_context(
    project: Option<PathBuf>,
    attach: Option<String>,
) -> anyhow::Result<(Option<ControlClient>, PathBuf)> {
    let registry = SessionRegistry::default_location()?;
    if let Some(attach) = attach {
        let client = registry.attach(parse_session_id(&attach)?)?;
        return Ok((
            Some(client.clone()),
            client.descriptor().project_root.clone(),
        ));
    }
    let project = match project {
        Some(project) => resolve_project_path(project)?,
        None => discover_project_root(&std::env::current_dir()?)?,
    };
    let sessions = registry.live(Some(&project))?;
    match sessions.as_slice() {
        [] => Ok((None, project)),
        [session] => Ok((Some(registry.attach(session.session_id)?), project)),
        _ => Ok((None, project)),
    }
}

fn resolve_project_path(path: PathBuf) -> anyhow::Result<PathBuf> {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    path.canonicalize()
        .with_context(|| format!("project path '{}' does not exist", path.display()))
}

/// Discover the canonical Aimer project root by walking upward for
/// `Aimer.toml`.
pub(crate) fn discover_project_root(start: &Path) -> anyhow::Result<PathBuf> {
    crate::commands::mcp_install::discover_project_root(start)
}

impl SessionDescriptor {
    fn status_name(&self) -> &'static str {
        match self.status {
            crate::session::SessionStatus::Starting => "starting",
            crate::session::SessionStatus::Locking => "locking",
            crate::session::SessionStatus::Fetching => "fetching",
            crate::session::SessionStatus::Compiling => "compiling",
            crate::session::SessionStatus::Building => "building",
            crate::session::SessionStatus::Launching => "launching",
            crate::session::SessionStatus::Running => "running",
            crate::session::SessionStatus::Idling => "idling",
            crate::session::SessionStatus::Restarting => "restarting",
            crate::session::SessionStatus::Stopping => "stopping",
            crate::session::SessionStatus::Stopped => "stopped",
            crate::session::SessionStatus::Error => "error",
        }
    }
}

/// Parse an exact session ID for the CLI.
pub fn parse_session_id(value: &str) -> anyhow::Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("invalid session ID '{value}'"))
}

/// Attach to an exact session. This helper is kept separate so tests and
/// future non-MCP adapters can use the same identity validation.
pub fn attach_exact(value: &str) -> anyhow::Result<ControlClient> {
    let registry = SessionRegistry::default_location()?;
    registry.attach(parse_session_id(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_payload_omits_unspecified_fields() {
        let payload = logs_payload(LogsArguments {
            stream: Some("app".to_string()),
            ..LogsArguments::default()
        });
        assert_eq!(payload["stream"], "app");
        assert!(payload.get("cursor").is_none());
    }

    #[test]
    fn log_payload_treats_all_as_the_default_stream() {
        let payload = logs_payload(LogsArguments {
            stream: Some("all".to_string()),
            limit: Some(5),
            ..LogsArguments::default()
        });

        assert!(payload.get("stream").is_none());
        assert_eq!(payload["limit"], 5);
    }

    #[test]
    fn global_mcp_discovery_uses_the_nearest_aimer_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let nested = project.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(project.join("Aimer.toml"), "[package]\nname = \"demo\"\n").unwrap();

        assert_eq!(
            discover_project_root(&nested).unwrap(),
            project.canonicalize().unwrap()
        );
    }

    #[test]
    fn global_mcp_discovery_rejects_directories_without_aimer_manifest() {
        let temp = tempfile::tempdir().unwrap();

        let error = discover_project_root(temp.path()).unwrap_err().to_string();

        assert!(error.contains("no Aimer.toml found"));
    }

    #[test]
    fn encode_returns_structured_content_and_actionable_check_errors() {
        let success = encode(
            "aimer_rust_search_symbols",
            Ok(json!({ "results": [], "truncated": false })),
        );
        assert_eq!(
            success.structured_content,
            Some(json!({ "results": [], "truncated": false }))
        );
        assert_eq!(success.is_error, Some(false));

        let error = encode(
            "aimer_rust_check",
            Err(anyhow!(
                "rust-analyzer did not start flycheck within 30 seconds"
            )),
        );
        assert_eq!(error.is_error, Some(true));
        assert_eq!(
            error.structured_content,
            Some(json!({
                "error": {
                    "code": "check_start_timeout",
                    "operation": "aimer_rust_check",
                    "phase": "start",
                    "message": "rust-analyzer did not start flycheck within 30 seconds"
                }
            }))
        );
    }

    #[test]
    fn symbol_search_order_is_repeatable_and_keeps_exact_matches_first() {
        let symbols = vec![
            json!({
                "name": "WidgetBuilder",
                "kind": 12,
                "location": { "uri": "file:///workspace/src/b.rs", "range": { "start": { "line": 4, "character": 0 } } }
            }),
            json!({
                "name": "Widget",
                "kind": 23,
                "location": { "uri": "file:///workspace/src/z.rs", "range": { "start": { "line": 8, "character": 0 } } }
            }),
            json!({
                "name": "WidgetAdapter",
                "kind": 5,
                "location": { "uri": "file:///workspace/src/c.rs", "range": { "start": { "line": 2, "character": 0 } } }
            }),
            json!({
                "name": "Widget",
                "kind": 23,
                "location": { "uri": "file:///workspace/src/a.rs", "range": { "start": { "line": 3, "character": 0 } } }
            }),
        ];
        let mut forward = symbols.clone();
        let mut reverse = symbols.into_iter().rev().collect::<Vec<_>>();

        sort_rust_symbols(&mut forward, "Widget");
        sort_rust_symbols(&mut reverse, "Widget");

        assert_eq!(forward, reverse);
        assert!(rust_symbol_is_exact_match(&forward[0], "Widget"));
        assert!(rust_symbol_is_exact_match(&forward[1], "Widget"));
        assert_eq!(forward[0]["location"]["uri"], "file:///workspace/src/a.rs");
        assert!(!rust_symbol_is_exact_match(&forward[2], "Widget"));
        assert_eq!(
            serde_json::to_string(&forward).unwrap(),
            serde_json::to_string(&reverse).unwrap()
        );
    }
}

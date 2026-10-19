use super::*;

#[tool_router(router = edit_tool_router, vis = "pub(super)")]
impl AimerMcpServer {
    #[tool(
        output_schema = mcp_result_schema(),
        name = "get_code_actions",
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
        encode("get_code_actions", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "rename_symbol",
        description = "Prepare a semantic symbol rename and return a preview. Possible unedited identifiers in macro_rules! bodies are listed separately and are never added automatically; apply the preview with apply_edit."
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
        encode("rename_symbol", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "preview_code_action",
        description = "Resolve a code action and return a preview; apply it with apply_edit."
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
        encode("preview_code_action", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "preview_edit",
        description = "Preview exact text edits and file create/delete operations in the selected Rust project. Edit anchors must match exactly once; the tool returns bounded diffs and a preview_id. Apply a reviewed preview with apply_edit."
    )]
    async fn rust_preview_edit(
        &self,
        Parameters(arguments): Parameters<RustPreviewEditArguments>,
    ) -> CallToolResult {
        let result = self
            .rust_analyzer
            .prepare_source_edit_preview(arguments.changes)
            .await;
        encode("preview_edit", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "format",
        description = "Format a Rust file or range and return a preview; apply it with apply_edit."
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
        encode("format", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "apply_edit",
        description = "Apply a previously reviewed Rust code, source-edit, or Cargo.toml preview if its source files are unchanged and any required manifest validation passed."
    )]
    async fn rust_apply_edit(
        &self,
        Parameters(arguments): Parameters<RustApplyEditArguments>,
    ) -> CallToolResult {
        let result = self
            .rust_analyzer
            .apply_preview(&arguments.preview_id)
            .await;
        encode("apply_edit", result)
    }
}

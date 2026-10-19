use super::*;

#[tool_router(router = navigation_tool_router, vis = "pub(super)")]
impl AimerMcpServer {
    #[tool(
        output_schema = mcp_result_schema(),
        name = "get_call_hierarchy",
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
        encode("get_call_hierarchy", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
        name = "expand_macro",
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
        encode("expand_macro", result)
    }
}

use super::*;

#[tool_router(router = cargo_tool_router, vis = "pub(super)")]
impl AimerMcpServer {
    #[tool(
        name = "get_manifest",
        description = "Read a Cargo.toml as compact markdown. Omit manifest_path to read the selected Rust project's Cargo.toml."
    )]
    async fn rust_get_manifest(
        &self,
        Parameters(arguments): Parameters<ManifestReadArguments>,
    ) -> CallToolResult {
        let result = read_manifest(&self.rust_analyzer.project_root(), arguments).await;
        encode("get_manifest", result)
    }

    #[tool(
        name = "get_project_metadata",
        description = "Return a compact Cargo workspace and package summary. Set detail to full to include complete bounded dependency and resolution metadata."
    )]
    async fn rust_get_project_metadata(
        &self,
        Parameters(arguments): Parameters<RustProjectMetadataArguments>,
    ) -> CallToolResult {
        let result = read_project_metadata_with_detail(
            &self.rust_analyzer.project_root(),
            self.rust_analyzer.allow_network(),
            arguments.detail,
        )
        .await
        .and_then(|value| metadata_failure(value, self.rust_analyzer.allow_network()));
        encode("get_project_metadata", result)
    }

    #[tool(
        name = "preview_manifest_edit",
        description = "Preview ordered set/remove operations on a Cargo.toml key path. The candidate is parsed and checked with Cargo metadata in a temporary workspace; use apply_edit to apply an applicable preview."
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
        encode("preview_manifest_edit", result)
    }

    #[tool(
        name = "cargo_check",
        description = "Run cargo check for the selected workspace or package. Returns Cargo's actual exit code and bounded output; filters and counts rustc warnings and routine Compiling/Checking/Finished progress lines. Cargo runs offline unless the existing rust-analyzer network setting opts in."
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
        encode("cargo_check", result)
    }

    #[tool(
        name = "cargo_test",
        description = "Run cargo test for the selected workspace or package, optionally with a test name filter. Returns Cargo's actual exit code and bounded output; filters and counts rustc warnings and routine Compiling/Checking/Finished progress lines. Cargo runs offline unless the existing rust-analyzer network setting opts in."
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
        encode("cargo_test", result)
    }

    #[tool(
        name = "cargo_run",
        description = "Build and run the selected project's default binary, a named binary, or an example. Program arguments are passed after `--`. Runs in the foreground with a 60-second default timeout (up to 300 seconds); returns bounded program output and the real exit code. Rust/build-script warnings and routine Cargo build progress are filtered and counted. Cargo runs offline unless the existing rust-analyzer network setting opts in."
    )]
    async fn rust_cargo_run(
        &self,
        Parameters(arguments): Parameters<CargoRunToolArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = run_cargo_binary(
            &self.rust_analyzer.project_root(),
            self.rust_analyzer.allow_network(),
            arguments,
            Some(&progress),
        )
        .await;
        encode("cargo_run", result)
    }

    #[tool(
        name = "check",
        description = "Run cargo clippy --workspace -- -D warnings directly. Returns Cargo's actual exit code and bounded structured diagnostics; routine Compiling/Checking/Finished progress lines are filtered and counted. Cargo runs offline unless user config opts in to network access."
    )]
    async fn rust_check(
        &self,
        Parameters(arguments): Parameters<RustDiagnosticsArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let progress = McpProgressReporter::from_context(&context);
        let result = run_clippy(
            &self.rust_analyzer.project_root(),
            self.rust_analyzer.allow_network(),
            arguments.file.as_deref().map(Path::new),
            Some(&progress),
        )
        .await;
        encode("check", result)
    }
}

/// Turn a `status: unavailable` metadata result into a tool error, so agents
/// see a failed call instead of a successful one carrying an error message.
fn metadata_failure(value: Value, allow_network: bool) -> anyhow::Result<Value> {
    if value.get("status").and_then(Value::as_str) != Some("unavailable") {
        return Ok(value);
    }
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("cargo metadata failed")
        .trim();
    let exit = value
        .get("exit_code")
        .and_then(Value::as_i64)
        .map(|code| format!(" (exit code {code})"))
        .unwrap_or_default();
    let hint = if !allow_network && message.contains("--offline") {
        "\nCargo runs offline for Aimer MCP tools. Run `cargo fetch` in the project, or set `allow_network = true` under `[mcp.rust_analyzer]` in ~/.aimer/config.toml."
    } else {
        ""
    };
    Err(anyhow!("cargo metadata failed{exit}: {message}{hint}"))
}

#[cfg(test)]
mod metadata_failure_tests {
    use super::metadata_failure;
    use serde_json::json;

    #[test]
    fn ready_metadata_passes_through() {
        let value = json!({ "status": "ready", "workspace_root": "." });
        assert_eq!(metadata_failure(value.clone(), false).unwrap(), value);
    }

    #[test]
    fn unavailable_metadata_becomes_an_error_with_an_offline_hint() {
        let value = json!({
            "status": "unavailable",
            "exit_code": 101,
            "message": "error: failed to download `x`\n  attempting to make an HTTP request, but --offline was specified\n"
        });
        let message = format!("{:#}", metadata_failure(value, false).unwrap_err());
        assert!(
            message.starts_with("cargo metadata failed (exit code 101): error: failed to download")
        );
        assert!(message.contains("cargo fetch"));
    }

    #[test]
    fn offline_hint_is_omitted_when_network_is_allowed() {
        let value = json!({ "status": "unavailable", "message": "--offline was specified" });
        let message = format!("{:#}", metadata_failure(value, true).unwrap_err());
        assert!(!message.contains("cargo fetch"));
    }
}

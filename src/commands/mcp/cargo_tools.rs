use super::*;

#[tool_router(router = cargo_tool_router, vis = "pub(super)")]
impl AimerMcpServer {
    #[tool(
        output_schema = mcp_result_schema(),
        name = "get_manifest",
        description = "Read a Cargo.toml as structured JSON. Omit manifest_path to read the selected Rust project's Cargo.toml."
    )]
    async fn rust_get_manifest(
        &self,
        Parameters(arguments): Parameters<ManifestReadArguments>,
    ) -> CallToolResult {
        let result = read_manifest(&self.rust_analyzer.project_root(), arguments).await;
        encode("get_manifest", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
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
        .await;
        encode("get_project_metadata", result)
    }

    #[tool(
        output_schema = mcp_result_schema(),
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
        output_schema = mcp_result_schema(),
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
        output_schema = mcp_result_schema(),
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
        output_schema = mcp_result_schema(),
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
        output_schema = mcp_result_schema(),
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

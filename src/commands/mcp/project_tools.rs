use super::*;

#[tool_router(router = project_tool_router, vis = "pub(super)")]
impl AimerMcpServer {
    #[tool(
        name = "status",
        description = "Return the current Aimer run status and app process identity. When several agents share the workspace, also lists connected clients and who holds the write, build and session locks."
    )]
    fn status(&self) -> CallToolResult {
        let result = self.request_live("status", Value::Null).map(|mut status| {
            if let (Some(map), Some(workspace)) = (status.as_object_mut(), self.gate.describe()) {
                map.insert("workspace".to_owned(), workspace);
            }
            status
        });
        encode("status", result)
    }

    #[tool(
        name = "restart_project",
        description = "Restart the attached Aimer project and wait for the new app run to start."
    )]
    fn restart_project(&self) -> CallToolResult {
        encode("restart_project", self.request_live("restart", Value::Null))
    }

    #[tool(
        name = "stop_project",
        description = "Stop the attached Aimer project."
    )]
    fn stop_project(&self) -> CallToolResult {
        encode("stop_project", self.request_live("stop", Value::Null))
    }

    #[tool(
        name = "get_logs",
        description = "Read bounded Aimer build, app, or system logs with cursor pagination."
    )]
    fn get_logs(&self, Parameters(arguments): Parameters<LogsArguments>) -> CallToolResult {
        encode(
            "get_logs",
            self.request_live("logs", logs_payload(arguments)),
        )
    }

    #[tool(
        name = "clear_logs",
        description = "Clear retained Aimer logs and return the sequence boundary that was cleared."
    )]
    fn clear_logs(&self, Parameters(arguments): Parameters<LogsArguments>) -> CallToolResult {
        encode(
            "clear_logs",
            self.request_live("clear_logs", logs_payload(arguments)),
        )
    }

    #[tool(
        name = "get_resource_usage",
        description = "Return the latest automatic CPU, memory, PID, thread, and GPU resource sample for the attached running Aimer app."
    )]
    fn get_resource_usage(&self) -> CallToolResult {
        encode(
            "get_resource_usage",
            self.request_live("resource_usage", Value::Null),
        )
    }

    #[tool(
        name = "set_resource_monitor_interval",
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
            "set_resource_monitor_interval",
            self.request_live("set_resource_interval", payload),
        )
    }

    #[tool(
        name = "record_resource_usage",
        description = "Record a bounded window of the attached app's resource usage. Takes duration in seconds (0.5–300) and interval in seconds (0.5–3), temporarily changes the live sampling interval, restores it afterward, and returns samples plus the run recording ID."
    )]
    async fn record_resource_usage(
        &self,
        Parameters(arguments): Parameters<ResourceCaptureArguments>,
    ) -> CallToolResult {
        let result = self.capture_resource_usage(arguments).await;
        encode("record_resource_usage", result)
    }

    #[tool(
        name = "list_resource_recordings",
        description = "List retained per-run resource recordings for an Aimer project, including completed runs. Requires an Aimer.toml project."
    )]
    fn list_resource_recordings(
        &self,
        Parameters(arguments): Parameters<ResourceRecordingsArguments>,
    ) -> CallToolResult {
        let result = self
            .require_aimer_project()
            .and_then(|()| ResourceRecordStore::for_project(&self.project_root))
            .and_then(|store| {
                let retention_days = crate::config::resource_monitor_config().retention_days;
                let page = store.list(retention_days, arguments.cursor, arguments.limit)?;
                serde_json::to_value(page).context("encoding resource recording list")
            });
        encode("list_resource_recordings", result)
    }

    #[tool(
        name = "get_resource_recording",
        description = "Read cursor-paginated timestamped samples from a retained per-run resource recording. Requires an Aimer.toml project."
    )]
    fn get_resource_recording(
        &self,
        Parameters(arguments): Parameters<ResourceRecordingArguments>,
    ) -> CallToolResult {
        let result = self
            .require_aimer_project()
            .and_then(|()| ResourceRecordStore::for_project(&self.project_root))
            .and_then(|store| {
                let retention_days = crate::config::resource_monitor_config().retention_days;
                let page = store.read(
                    &arguments.recording_id,
                    arguments.cursor,
                    arguments.limit,
                    retention_days,
                )?;
                serde_json::to_value(page).context("encoding resource recording samples")
            });
        encode("get_resource_recording", result)
    }

    #[tool(
        name = "get_crate_docs",
        description = "Read documentation for a Rust crate or item from docs.rs, or installed rust-docs-json for std, core, alloc, proc_macro, and test. Trait lookups also return the trait signature, all method signatures, required/provided status, and each method's documentation. Uses Cargo.lock for registry crates. Toolchain crates use the active Rust toolchain when JSON docs are installed, otherwise installed nightly; version can select a toolchain. Caches registry documentation for offline use."
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
        encode("get_crate_docs", result)
    }

    #[tool(
        name = "search_crate_docs",
        description = "Search public Rust item names, paths, and documentation in a docs.rs crate or installed toolchain crate (std, core, alloc, proc_macro, test). Uses Cargo.lock for registry crates and the active toolchain, then installed nightly, for toolchain crates; version can select a toolchain. Returns bounded results with item paths and summaries."
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
        encode("search_crate_docs", result)
    }
}

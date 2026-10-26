[← Back to README](../README.md)

# Model Context Protocol (MCP) Integration

Aimer features native integration with the **Model Context Protocol (MCP)**, allowing AI coding assistants to observe,
diagnose, and control your running application directly.

## Supported AI Agents

- **Claude Code** (`claude` / `claude-code`)
- **Cursor** (`cursor`)
- **Codex** (`codex`)

## Installing MCP Integration

Install the Aimer MCP server entry directly into your agent's configuration:

```bash
# Configure for Claude Code
aimer mcp --install claude

# Configure for Cursor
aimer mcp --install cursor

# Configure for Codex
aimer mcp --install codex

# Install globally across all projects
aimer mcp --install claude --global

# Bind MCP to a regular Rust project
aimer mcp --install claude --project /path/to/rust-project
```

For a global install, start Claude Code from inside a Rust project containing `Cargo.toml` or an Aimer project
containing `Aimer.toml`. Rust navigation, editing, Cargo, and crate documentation tools work in either project type.
Aimer session controls need a live Aimer session; resource recording history needs an Aimer project.

## Available MCP Tools

Once connected, your AI assistant can execute the following tools:

- `status`: Inspect the current run state, process ID, runtime, and target device.
- `restart_project`: Trigger a clean restart and rebuild of the app, waiting for the new run.
- `stop_project`: Stop the running project session.
- `get_logs`: Fetch bounded build, app, or system logs with cursor pagination and source code locations.
- `clear_logs`: Clear retained logs up to a specific sequence boundary.
- `get_resource_usage`: Read the latest app CPU, GPU, memory, PID, and thread sample.
- `set_resource_monitor_interval`: Set the live sample interval to 500–3000 ms; omit `interval_ms` to restore the global default.
- `record_resource_usage`: Capture a bounded sample window with `duration` in seconds (0.5–300) and `interval` in seconds (0.5–3), then return the samples inline and the run recording ID.
- `list_resource_recordings`: List retained per-run resource recordings, including completed app runs.
- `get_resource_recording`: Read paginated samples from a prior run.
- `get_crate_docs`: Read crate or item documentation from docs.rs, or installed Rust toolchain documentation for `std`, `core`, `alloc`, `proc_macro`, and `test`. A trait lookup includes its rendered declaration, method signatures, required/provided status, and each method's documentation. The tool resolves an omitted version from the selected Rust project's `Cargo.lock` and caches the crate's Rustdoc JSON index for offline lookups.
- `search_crate_docs`: Search public item names, paths, and documentation text within a crate. Results include item paths and short documentation summaries; pass a result's path to `get_crate_docs` to read the full item documentation.
- `start_lsp`: Load the embedded rust-analyzer engine for `project_path`; waits for workspace loading with progress notifications and returns ready status. `project_path` may be an absolute path or a path relative to the current Rust project. Omitting it uses the current project.
- `search_symbols`, `search_text`, `inspect_file`, `get_document_symbols`, and `completion`: Search Rust workspace symbols and project text, inspect structured bounded file ranges alongside literal or regex matches (or numbered plain text with `format: "text"`), inspect a compact filtered symbol outline, or get up to 50 completion suggestions without documentation by default. Symbol searches accept Rust kinds such as `trait`, `impl`, and `macro` and return crate, module, and re-export metadata.
- `find_definition`, `find_declaration`, `type_definition`, `find_references`, `find_implementations`, `hover`, `get_call_hierarchy`, and `expand_macro`: Navigate Rust code using rust-analyzer's semantic model. Implementation results include deterministic ordering, labels, pagination, test filtering, and completeness metadata; `find_implementations` also accepts a qualified `symbol_path`.
- `signature_help`, `document_highlights`, `inlay_hints`, and `get_code_lenses`: Inspect call signatures, local symbol occurrences, inferred hints, and code lenses.
- `get_diagnostics` and `check`: Read native Rust diagnostics or run `cargo clippy --workspace -- -D warnings`, returning Cargo's actual exit code and bounded diagnostics.
- `get_code_actions`, `prepare_rename`, `rename_symbol`, `preview_code_action`, `format`, and `apply_edit`: Prepare bounded edit previews and apply a preview after review.
- `preview_edit`: Preview project-relative exact-text edits, new files, and file deletions. Each `old_text` anchor must occur exactly once; overlapping or ambiguous edits are rejected. Rust files include a syntax parse report. Review the bounded diff, then apply its `preview_id` with `apply_edit`. Use `preview_manifest_edit` for `Cargo.toml`, which runs Cargo validation.
- `get_manifest` and `get_project_metadata`: Read parsed `Cargo.toml` data or a compact summary of the selected workspace's packages, targets, and direct dependencies. Pass `detail: "full"` to `get_project_metadata` for full bounded Cargo dependency and resolution metadata. It uses `--locked` and does not update the project's lockfile.
- `preview_manifest_edit`: Preview ordered `set` or `remove` operations on a `Cargo.toml` key path. The tool parses the candidate and runs Cargo metadata in a temporary workspace; invalid previews cannot be applied. Apply an applicable preview with `apply_edit`.
- `cargo_check` and `cargo_test`: Run Cargo check or test for the selected workspace or a package and return the actual exit code with bounded stdout and stderr. Rustc warning diagnostics are filtered from the output and counted in `warnings_filtered`; routine `Compiling`, `Checking`, and `Finished` lines are filtered and counted in `cargo_progress_lines_filtered`. Other compiler diagnostics and test output remain visible. Both tools accept target and feature selection; test also accepts a test-name filter. `locked` defaults to Cargo's normal behavior, and the result reports whether `Cargo.lock` changed. Each call has a five-minute timeout.
- `cargo_run`: Build and run the selected project's default binary, a named binary, or an example. Pass application arguments in `program_args`; set `timeout_seconds` from 1 to 300 (default 60). The bounded result preserves program output and the real exit code, while filtering and counting Rust/build-script warnings and routine Cargo build progress.

Rust code tools load the embedded IDE engine on the first request. A custom proc-macro helper path can override the bundled helper, and Cargo network policy can be set in `~/.aimer/config.toml`:

```toml
[mcp.rust_analyzer]
# Optional; overrides the helper bundled in Aimer.
proc_macro_server_path = "/path/to/rust-analyzer-proc-macro-srv"
# Defaults to false. When false, Cargo calls made by Rust MCP tools run offline.
allow_network = false
```

The embedded engine loads lazily when a Rust tool is first called, or explicitly through `start_lsp`. It updates indexed Rust files as they change and reloads the Cargo workspace after manifest, build-script, or source-structure changes. Procedural macros run in the bundled matching helper process; Aimer extracts it to a private temporary directory for the lifetime of the MCP process. Builds without `bundled-proc-macro` need a helper beside `aimer` or a configured `proc_macro_server_path`. `check` runs Clippy directly with all warnings denied; `cargo_check` and `cargo_test` run their corresponding Cargo commands; `cargo_run` builds and runs a binary or example in the foreground. These Cargo tools return bounded results and filter routine build progress; `cargo_run` also filters and counts Rust/build-script warnings. Code edits are returned as previews and are only written after `apply_edit`; stale previews are rejected. Manifest previews preserve existing TOML comments and formatting, and applying one schedules an embedded workspace reload on the next Rust request. Cargo commands run offline unless the network setting opts in. Existing docs.rs tools retain their separate network and cache behavior.

Range formatting uses rustfmt's unstable file-range support and invokes
`rustup run nightly rustfmt`; whole-file formatting uses the toolchain's regular
`rustfmt`.

The same documentation features are available directly from the CLI. It prints JSON, which makes the output easy to pipe into other tools:

```bash
# Read a crate's root documentation or a specific item
aimer docs serde
aimer docs serde de::Deserialize

# Search item names, paths, and documentation in a crate
aimer docs serde --search Deserialize --limit 10

# Select a version when it is not in the current project's Cargo.lock
aimer docs serde --search Deserialize --version 1.0.229
```

For standard-library documentation, install Rust's JSON documentation component once:

```bash
rustup component add --toolchain nightly rust-docs-json

aimer docs std vec::Vec
aimer docs std iter::Iterator --version nightly
aimer docs std --search HashMap --limit 10
aimer docs core option::Option
```

The MCP tools accept the same crate names and item paths (for example, `crate_name: "std"`, `item_path: "vec::Vec"`). Toolchain crates do not require `Cargo.lock` or docs.rs. Aimer reads `share/doc/rust/json` from the project's active toolchain when available, otherwise from an installed nightly toolchain. Set `version` to a Rust toolchain such as `nightly` or `nightly-2026-10-01` to choose one explicitly. Results report the documentation's actual compiler version and `source: "rust-toolchain"`; when falling back to nightly, its APIs may differ from the project's toolchain. Re-exports from `core` and `alloc` resolve under their public `std` paths. These reads work offline, and `refresh` has no effect because the installed files are read on every request.

For trait results, the top-level `documentation` field contains the expanded trait description. `trait_details` contains `signature`, a Rust-like `definition`, and `methods` entries with each method's `name`, `signature`, `required` flag, and `documentation`.

The docs.rs cache is stored under `~/.aimer/docs/docs.rs`, with each crate version in its own subdirectory. It keeps the original gzip download and a zstd-compressed JSON copy for offline reads. Documentation is fetched on first use, so an uncached crate version requires an internet connection once.

## Resource monitoring

The TUI's Resource Monitor pane is available with `4` or `Tab`. Aimer samples the running app automatically and records each run under `~/.aimer/resource-recordings`; completed recordings are kept for 30 days by default and can be read later with the MCP tools above. The `record_resource_usage` tool temporarily applies the requested interval, waits for the duration, restores the previous interval, and returns that window's samples inline. It also returns the run recording ID for later paginated reads. The host collector uses `sysinfo` for process CPU and memory on macOS, Windows, and Linux. macOS GPU usage is collected from per-process Apple GPU records when available; Android process readings use the connected device through `adb`. iOS device process counters are unavailable to the host because of device sandboxing. Unavailable fields include a reason in the TUI and recorded data.

Resource settings live in the global `~/.aimer/config.toml`:

```toml
[resource_monitor]
sample_interval_ms = 1000 # 500–3000 ms
retention_days = 30       # 1–3650 days
```

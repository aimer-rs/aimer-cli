[← Back to README](../README.md)

# Model Context Protocol (MCP) Integration

Aimer features native integration with the **Model Context Protocol (MCP)**, allowing AI coding assistants to observe,
diagnose, and control your running application directly.

## Supported AI Agents

| Agent | `--install` name | Configuration file |
|---|---|---|
| Claude Code | `claude` / `claude-code` | `~/.claude.json` |
| Codex | `codex` | `~/.codex/config.toml` (`$CODEX_HOME` is honored) |
| Cursor | `cursor` | `~/.cursor/mcp.json` |
| Gemini CLI | `gemini-cli` | `~/.gemini/settings.json` |
| Windsurf | `windsurf` | `~/.codeium/windsurf/mcp_config.json` |
| Qwen Code | `qwen-code` | `~/.qwen/settings.json` |
| Kiro CLI | `kiro-cli` | `~/.kiro/settings/mcp.json` |
| Junie | `junie` | `~/.junie/mcp/mcp.json` |
| iFlow CLI | `iflow-cli` | `~/.iflow/settings.json` |
| Augment | `augment` | `~/.augment/settings.json` |
| CodeBuddy | `codebuddy` | `~/.codebuddy/mcp.json` |
| Kimi Code CLI | `kimi-code-cli` | `~/.kimi/mcp.json` |
| Droid | `droid` | `~/.factory/mcp.json` |
| GitHub Copilot CLI | `github-copilot` | `~/.copilot/mcp-config.json` |
| OpenCode | `opencode` | `~/.config/opencode/opencode.json` |
| Crush | `crush` | `~/.config/crush/crush.json` |
| Amp | `amp` | `~/.config/amp/settings.json` |

Run `aimer mcp --install` without an agent to print this list.

## Installing MCP Integration

Install the Aimer MCP server entry directly into your agent's user-level configuration. The installation is global:
it is not tied to any project, and there is no per-project installation.

```bash
# Configure for Claude Code
aimer mcp --install claude

# Configure for Cursor
aimer mcp --install cursor

# Configure for Codex
aimer mcp --install codex

# Configure several agents at once
aimer mcp --install claude,cursor,codex

# List the supported agents
aimer mcp --install
```

Existing settings and other MCP servers in an agent's file are preserved; only the `aimer` entry is written. The file
must be strict JSON (no comments).

To only see the configuration without changing any agent's files, print it instead:

```bash
# JSON entry (`mcpServers`), the format used by Claude Code and Cursor
aimer mcp --json

# TOML entry (`mcp_servers`), the format used by Codex
aimer mcp --toml
```

`--json` and `--toml` write only to stdout. They cannot be combined with each other, `--install`, `--project`, or
`--attach`.

The installed entry runs `aimer mcp` without a project path. Start your agent from inside a Rust project containing
`Cargo.toml` or an Aimer project containing `Aimer.toml`. Rust navigation, editing, Cargo, and crate documentation
tools work in either project type.
Aimer session controls need a live Aimer session; resource recording history needs an Aimer project.

## Available MCP Tools

Once connected, your AI assistant can execute the following tools:

- `status`: Inspect the current run state, process ID, runtime, and target device.
- `list_sessions`, `attach_session`, and `detach_session`: Choose which live Aimer session this connection controls. `list_sessions` shows the live sessions of the workspace (or of every project with `all_projects`), marks the attached one, and reports the mode: `automatic`, `pinned`, or `detached`. `attach_session` with a `session_id` pins that session; without one it goes back to automatic. `detach_session` controls no session until the next attach. They change only the calling connection, so agents sharing a workspace host do not move each other. See [Choosing the session](#choosing-the-session).
- `restart_project`: Trigger a clean restart and rebuild of the app, waiting for the new run.
- `stop_project`: Stop the running project session.
- `get_logs`: Fetch bounded build, app, or system logs with cursor pagination and source code locations.
- `clear_logs`: Clear retained logs up to a specific sequence boundary.
- `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, `mouse_move`, `left_click_drag`, `scroll`, `key`, and `type`: Drive the running app like a mouse and keyboard — the mouse glides to its targets along a curved, eased path rather than jumping, and a command sent mid-glide retargets it — in the vocabulary of the computer-use tools. Coordinates are `[x, y]` logical pixels from the top-left corner of the app's window; a click tool's `coordinate` is optional (without it the click happens wherever the mouse already is, once an earlier click, `mouse_move` or `scroll` has placed it), `scroll` is a smooth trackpad-style swipe and takes `scroll_direction`, `scroll_amount` (steps of 40 pixels) and an optional `coordinate` (without it, it scrolls wherever the mouse already is), `key` takes `text` such as `Return` or `ctrl+a`, and `type` types `text` a keystroke at a time at a human pace (a long text is typed faster, within about three seconds). The app shows a badge for as long as an agent holds the app: the MCP server tells the session an agent is remote-controlling it from its first app tool until it exits, and a one-shot `aimer input` or `aimer screenshot` counts for four seconds after it. `aimer run`'s own connection never shows it. They are listed only while an app is connected to the session — it has announced its input server, the session has reached it, and it is still running — not merely while the MCP server is attached; the server sends a tool-list-changed notification when that changes, and a client that ignores it sees them on its next listing. They need the attached app to accept input; see [Driving the app](#driving-the-app).
- `tap`, `swipe`, and `pinch`: The touch gestures for an app on a phone or tablet.
- `screenshot`: See the running app: a picture of its window as the app itself drew it, with nothing else on the screen. A pixel in the picture is a logical pixel by default, so its coordinates can be passed straight to the click, scroll and drag tools; `max_width` and `jpeg_quality` make it smaller, and the reply says what to divide by. Use it to check the result of an action.
- `send_input`: Send raw `aimer_prendre` protocol lines for events the tools above do not cover (`back`, `link`, `menu`, `file-drop`, `preedit`, `quit`, ...). The whole request is checked before anything is sent.
- `get_resource_usage`: Read the latest app CPU, GPU, memory, PID, and thread sample.
- `set_resource_monitor_interval`: Set the live sample interval to 500–3000 ms; omit `interval_ms` to restore the global default.
- `record_resource_usage`: Capture a bounded sample window with `duration` in seconds (0.5–300) and `interval` in seconds (0.5–3), then return the samples inline and the run recording ID.
- `list_resource_recordings`: List retained per-run resource recordings, including completed app runs.
- `get_resource_recording`: Read paginated samples from a prior run.
- `get_crate_docs`: Read crate or item documentation from docs.rs, or installed Rust toolchain documentation for `std`, `core`, `alloc`, `proc_macro`, and `test`. A trait lookup includes its rendered declaration, method signatures, required/provided status, and each method's documentation. The tool resolves an omitted version from the selected Rust project's `Cargo.lock` and caches the crate's Rustdoc JSON index for offline lookups.
- `search_crate_docs`: Search public item names, paths, and documentation text within a crate. Results include item paths and short documentation summaries; pass a result's path to `get_crate_docs` to read the full item documentation.
- `start_lsp`: Load the embedded rust-analyzer engine for `project_path`; waits for workspace loading with progress notifications and returns ready status. `project_path` may be an absolute path or a path relative to the current Rust project. Omitting it uses the current project.
- `search_symbols`, `search_text`, `inspect_file`, `get_document_symbols`, and `completion`: Search Rust workspace symbols and project text, inspect bounded file ranges alongside literal or regex matches as numbered plain text (or structured JSON with `format: "json"`), inspect a compact filtered symbol outline, or get up to 50 completion suggestions without documentation by default. Symbol searches accept Rust kinds such as `trait`, `impl`, and `macro` and return crate, module, and re-export metadata.
- `find_definition`, `find_declaration`, `type_definition`, `find_references`, `find_implementations`, `hover`, `get_call_hierarchy`, and `expand_macro`: Navigate Rust code using rust-analyzer's semantic model. Implementation results include deterministic ordering, labels, pagination, test filtering, and completeness metadata; `find_implementations` also accepts a qualified `symbol_path`.
- `signature_help`, `document_highlights`, `inlay_hints`, and `get_code_lenses`: Inspect call signatures, local symbol occurrences, inferred hints, and code lenses.
- `get_diagnostics` and `check`: Read native Rust diagnostics or run `cargo clippy --workspace -- -D warnings`, returning Cargo's actual exit code and bounded diagnostics.
- `get_code_actions`, `prepare_rename`, `rename_symbol`, `preview_code_action`, `format`, and `apply_edit`: Prepare bounded edit previews and apply a preview after review.
- `preview_edit`: Preview project-relative exact-text edits, new files, and file deletions. Each `old_text` anchor must occur exactly once; overlapping or ambiguous edits are rejected. Rust files include a syntax parse report. Review the bounded diff, then apply its `preview_id` with `apply_edit`. Use `preview_manifest_edit` for `Cargo.toml`, which runs Cargo validation.
- `get_manifest` and `get_project_metadata`: Read parsed `Cargo.toml` data or a compact summary of the selected workspace's packages, targets, and direct dependencies. Pass `detail: "full"` to `get_project_metadata` for full bounded Cargo dependency and resolution metadata. It uses `--locked` and does not update the project's lockfile.
- `preview_manifest_edit`: Preview ordered `set` or `remove` operations on a `Cargo.toml` key path. The tool parses the candidate and runs Cargo metadata in a temporary workspace; invalid previews cannot be applied. Apply an applicable preview with `apply_edit`.
- `cargo_check` and `cargo_test`: Run Cargo check or test for the selected workspace or a package and return the actual exit code with bounded stdout and stderr. Rustc warning diagnostics are filtered from the output and counted in `warnings_filtered`; routine `Compiling`, `Checking`, and `Finished` lines are filtered and counted in `cargo_progress_lines_filtered`. Other compiler diagnostics and test output remain visible. Both tools accept target and feature selection; test also accepts a test-name filter. `locked` defaults to Cargo's normal behavior, and the result reports whether `Cargo.lock` changed. Each call has a five-minute timeout.
- `cargo_run`: Build and run the selected project's default binary, a named binary, or an example. Pass application arguments in `program_args`; set `timeout_seconds` from 1 to 300 (default 60). The bounded result preserves program output and the real exit code, while filtering and counting Rust/build-script warnings and routine Cargo build progress.

### Output format

Tool results are compact markdown that keeps only the essential information:

- Each diagnostic is shown as its `file:line:column` location followed by the compiler's rendered text; the separate `line`, `column`, `code`, `message`, `severity` and `source` fields are dropped.
- Zero counts (`warning_count`, `*_filtered`, `duration_ms`), `false` `*_truncated` flags, empty `stdout`/`stderr`, and the top-level `operation` (the tool name) are omitted.
- `stderr` keeps errors; Cargo's "generated N warnings" summary lines are removed.
- Search and navigation results are listed one per line, not as a table:
  - `search_text`: `- path/to/file.rs:line:column matched text`
  - `search_symbols`: `- path/to/file.rs:line:column kind name (module_path)`, with `[reexport]` appended for re-exports
  - `find_definition`, `find_declaration`, `find_references`, `type_definition`: `- path/to/file.rs:line:column`
  - `find_implementations`: `- path/to/file.rs:line impl header`, with `[test]` appended for test implementations
- Small values are kept on one line instead of being spread over several:
  - A short list of single-line scalars is joined with commas, for example `- tags: a, b`. This applies when the joined text is at most 300 characters; longer lists print one item per line.
  - A small object whose values are all single-line scalars prints as `key=value, key=value`, for example `- range: end_line=7, start_line=3`. Keys appear in alphabetical order and null fields are skipped. This applies when the joined text is at most 120 characters.
  - The same rule applies to objects inside a list: a flat object becomes one `- key=value, key=value` line with no `[n]` index header, while an object with nested values or longer text keeps its `- [n]` header and nested bullets.
- Terminal colour codes and trailing blanks are stripped, and runs of blank lines are collapsed.
- Size limits keep a single call from flooding the agent: a text field over 4,000 characters keeps its head and tail with an `omitted` note, a list over 50 items is cut with a `<name>_omitted` count, and a whole response is capped at about 24,000 characters.

Example of the inline layout:

```markdown
- name: demo
- tags: a, b
- position: column=4, line=12
- dependencies:
  - name=serde, optional=false, version=1.0
  - name=tokio, optional=true, version=1.40
```

To get the full, unabridged output instead, start the server with `--full-output` or set `AIMER_MCP_FULL_OUTPUT=1` in the server's environment:

```bash
aimer mcp --full-output
```

### Sharing one workspace between agents

By default `aimer mcp` is a thin proxy. The first one for a workspace starts a background host that owns the Rust analysis engine; later ones attach to it, so several agents on the same workspace (for example Claude Code and Codex) share one engine instead of each indexing the project. The host listens on loopback only, requires a per-host token, keeps its record in `~/.aimer/mcp-hosts/` (directory `0700`, file `0600`), and exits about a minute after its last client leaves. Workspaces are keyed on their canonical root, so symlinked or nested paths resolve to the same host.

Tool calls follow a readers-writer rule, with a fair queue so a waiting write is not starved by new reads:

| Class | Tools | Rule |
|---|---|---|
| Read | search, find, hover, `inspect_file`, diagnostics, docs, `get_*`, `list_sessions`, `attach_session`, `detach_session` (they change only the calling connection), and every preview tool (`preview_edit`, `rename_symbol`, `format`, `preview_code_action`, `preview_manifest_edit`) | Any number at once |
| Build | `cargo_check`, `cargo_test`, `cargo_run`, `check` | Run beside reads, one at a time |
| Write | `apply_edit`, `start_lsp` | Runs alone |
| Session | `restart_project`, `stop_project`, `clear_logs`, `set_resource_monitor_interval`, `record_resource_usage`, and every input tool (`left_click`, `key`, `type`, `scroll`, `tap`, `send_input`, `screenshot`, ...) | One at a time; control the running app, not the workspace files |

A call that cannot get its lock within a minute (five for builds) returns a `busy` error that names the current holder; retry it shortly. Unlisted tools are treated as writes.

Previews and code actions belong to the client that created them: another client cannot apply them, and they are dropped when that client disconnects. A preview still fails if its files changed since it was made. `status` lists connected clients and lock holders when the workspace is shared.

A host belongs to the `aimer` binary that started it. After you rebuild or update `aimer`, the next `aimer mcp` starts a fresh host, so new tools appear without manual cleanup. The older host keeps serving the agents still connected to it and exits after about a minute without clients. An agent that is already connected keeps its old tool list until it reconnects its MCP server.

Output format (`--full-output`) is chosen per client. Pass `--standalone` to keep the old one-process-per-client behavior. `--attach` always serves standalone, because it is bound to one live session. If no host can be reached, `aimer mcp` falls back to standalone automatically.

### Choosing the session

Each agent connection controls at most one live Aimer session, and that choice can change while the connection lives. A connection is in one of three modes, reported by `list_sessions`:

| Mode | Behaviour |
|---|---|
| `automatic` | The default. When the workspace has exactly one live session, the connection attaches to it, including a session started with `aimer run` after the agent connected. A session that ends is let go. With several live sessions nothing is chosen for the agent, and a session already attached stays while it is alive. |
| `pinned` | `attach_session` with a `session_id`, or `aimer mcp --attach <ID>`. The connection stays on that session whatever else is alive. |
| `detached` | `detach_session`. No session is controlled, and none is picked automatically, until `attach_session` is called. |

`attach_session` without a `session_id` returns the connection to `automatic`. In automatic mode the registry is looked at again on session tool calls and about once a second, so a new session is found within a second. The tool list is refreshed with a tool-list-changed notification when the session or its app changes.

The choice belongs to one connection. Agents that share a workspace host each follow their own session, so one agent attaching, or detaching, does not move another. A session of another project can be attached by ID; the Rust, Cargo and documentation tools keep working on the workspace the server was started for.

### Rust engine configuration

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

## Driving the app

`aimer run` asks the app it launches to accept synthetic input, and the input tools above, and `aimer input` in a shell, send it. Nothing needs to change in the app: a debug build of an Aimer app starts the input server when the launcher sets `AIMER_PRENDRE`, and a release build never does.

```sh
aimer input click 120 48
aimer input key ctrl+a
aimer input type "hello"
aimer input scroll 200 300 down 3
aimer input send "back invoke" "link aimer://item/7"
```

```sh
aimer screenshot -o app.png          # the app's window, at its logical size
aimer screenshot --jpeg 70 --width 800
```

The picture is drawn by the app itself, so it holds only the app — no other window, no desktop — and needs no screen-recording permission. It works where the renderer can copy its frames out, which today is Metal on macOS and iOS; elsewhere the command says so at once.

`aimer input` sends to the only live session; pass `--session <id>` (see `aimer sessions`) when there are several.

How it reaches the app depends on where the app runs:

| Target | Route |
|---|---|
| macOS, desktop, iOS Simulator | Loopback. |
| iOS device | The device's `devicectl` tunnel address, so it works wired and wireless. The app listens beyond loopback for the run, which exposes it to its network with a random per-run token as the only protection. |

While the session is attached the app shows a `driven by aimer-cli` badge in the corner of its window, and draws where each injected pointer and finger is (the cursor icon for the mouse, which sways gently when it rests), so a screenshot of a driven app shows what the agent did. A debug build only.

Set `AIMER_PRENDRE=0` in the environment of `aimer run` to turn all of this off. Android is not wired yet.


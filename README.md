<div align="center">

```text
           #▄▄▄▄#░       
      x   ░▄▌   █        
     #▄▄#  █ xx █x       
   ░▄█▀▀█▄ █ x░ █░       
   x█ x ░▀▀▀ ░  █ X      
    █▄ x░░x ░x ░x█       
     █░ x░░░ x ░ █░      
   xx▀██▄ ░xx▒░ ▐▀       
        ▀█▒▓▒x▓█▀        
         █ ▓▒▓ █         
▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
```

# Aimer CLI (`aimer`)

**The official command-line toolchain and developer interface for
the [Aimer Framework](https://github.com/Cottons29/aimer).**

[![Rust Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)
[![Platform Support](https://img.shields.io/badge/platforms-macOS%20%7C%20iOS%20%7C%20Android%20%7C%20Linux%20%7C%20Windows%20%7C%20Web-brightgreen.svg)](#supported-platforms)

</div>

---

`aimer` is the developer command-line interface for the **Aimer Framework**  a modern, declarative, cross-platform GUI
framework built in Rust. It manages project lifecycle from scaffolding and live development to native compilation, asset
bundling, and release packaging across desktop, mobile, and web targets.

---

## Table of Contents

- [Features](#features)
- [Supported Platforms](#supported-platforms)
- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Quick Start](#quick-start)
    - [1. Create a New Project](#1-create-a-new-project)
    - [2. Project Layout](#2-project-layout)
    - [3. Run the Application](#3-run-the-application)
- [Command Reference](#command-reference)
    - [`aimer create`](#aimer-create)
    - [`aimer run`](#aimer-run)
    - [`aimer build`](#aimer-build)
    - [`aimer assemble`](#aimer-assemble)
    - [`aimer doctor`](#aimer-doctor)
    - [`aimer clean`](#aimer-clean)
    - [`aimer migrate`](#aimer-migrate)
    - [`aimer completions`](#aimer-completions)
    - [`aimer mcp`](#aimer-mcp)
    - [`aimer skill`](#aimer-skill)
    - [`aimer sessions`](#aimer-sessions)
- [Interactive Developer Console](#interactive-developer-console)
- [Model Context Protocol (MCP) Integration](#model-context-protocol-mcp-integration)
    - [Supported AI Agents](#supported-ai-agents)
    - [Installing MCP Integration](#installing-mcp-integration)
    - [Available MCP Tools](#available-mcp-tools)
- [Project Configuration (`Aimer.toml`)](#project-configuration-aimertoml)
- [Assets Management](#assets-management)
- [Hot Reload & Unstable Features](#hot-reload--unstable-features)
- [Workspace Architecture](#workspace-architecture)
- [Contributing & Development](#contributing--development)
- [License](#license)

---

## Features

- **Multi-Platform Native Support**: Seamlessly build and run apps on macOS, iOS, Android, Linux, Windows, and
  WebAssembly.
- **Interactive Developer TUI**: Rich terminal interface (via Ratatui) providing unified application logs, build
  outputs, widget inspection, live app resource usage, and hotkeys.
- **Live Device Discovery**: Automatically detects connected Android devices (`adb`), iOS devices/simulators (`xcrun`),
  host desktops, and web browsers.
- **Model Context Protocol (MCP)**: Native stdio MCP server for AI coding assistants (Claude Code, Cursor, Codex) to
  inspect runtime state, view logs, and restart apps.
- **Turnkey Platform Packaging**: One-command bundling (`aimer assemble`) producing macOS `.app`, Android `.apk`, iOS
  `.app`/`.ipa`, and Web distribution packages.
- **Environment Diagnostics**: Built-in `aimer doctor` checks installed compiler toolchains, platform SDKs, and Rust
  cross-compilation targets with remediation commands.
- **Dynamic Shell Completions**: Self-updating shell completion scripts for Bash, Zsh, and Fish that dynamically reflect
  the active CLI command tree.
- **WASM Hot Reload Pipeline**: Nightly WebAssembly shadow guest pipeline for rapid UI iteration without losing
  application state.

---

## Supported Platforms

| Platform    | Target Identifier               | Build Artifact                 | Requirement                                               |
|:------------|:--------------------------------|:-------------------------------|:----------------------------------------------------------|
| **macOS**   | `macos`                         | `.app` bundle                  | macOS, Xcode & Command Line Tools                         |
| **iOS**     | `ios` / `ios-simulator`         | iOS App Bundle / Simulator App | macOS, Xcode, `xcrun`                                     |
| **Android** | `android` / `android-simulator` | Android `.apk`                 | Android SDK, `adb`, `cargo-ndk`, Gradle                   |
| **Linux**   | `linux`                         | Portable binary bundle         | GCC/Clang, `pkg-config`                                   |
| **Windows** | `windows`                       | Portable executable bundle     | MSVC C++ Build Tools (`link.exe`)                         |
| **Web**     | `web`                           | WebAssembly + HTML/JS          | [`trunk`](https://trunkrs.dev/), `wasm32-unknown-unknown` |

---

## Prerequisites

Before using `aimer`, ensure you have a standard Rust toolchain installed:

- **Rust**: Rust compiler (`rustc`) and package manager (`cargo`), version 1.98 or newer (required by the embedded rust-analyzer engine).
- **Rustup**: For managing cross-compilation targets.

Run `aimer doctor` after installation to verify platform dependencies and receive tailored setup commands.

For Rust standard-library navigation and proc-macro crate analysis, install the
Rust source component:

```bash
rustup component add rust-src
```

---

## Installation

### From Source

Clone the repository and install the binary into your Cargo binary directory (`~/.cargo/bin`):

```bash
git clone https://github.com/Cottons29/aimer_cli.git
cd aimer_cli
rustup component add rustc-dev
cargo install --path .
```

The default build is the lightweight CLI: it does not include the `aimer mcp`
server, the embedded rust-analyzer engine, or the proc-macro helper, and it does
not need `rustc-dev`. Opt in with Cargo features:

| Feature | Adds |
| --- | --- |
| `mcp` | The `aimer mcp` server and the embedded rust-analyzer IDE engine. |
| `bundled-proc-macro` | Everything in `mcp`, plus the matching proc-macro helper bundled in the executable. |
| `legacy-lsp` | Everything in `mcp`, plus the legacy LSP-shaped MCP tools. |

```bash
rustup component add rustc-dev   # only needed for bundled-proc-macro
cargo install --path . --features bundled-proc-macro
```

With `bundled-proc-macro`, the build compiles the helper with
`RUSTC_BOOTSTRAP=1`, and at startup Aimer extracts it into a private temporary
directory and launches it as a child process. No separate helper installation
or `rust-analyzer` executable on `PATH` is needed. Builds with only `mcp` can
use a helper beside `aimer`, or a helper selected with `proc_macro_server_path`.

Verify the installation:

```bash
aimer --version
```

### Enable Shell Completions

Generate and install completions for your active shell:

```bash
# For Zsh
aimer completions zsh --install

# For Bash
aimer completions bash --install

# For Fish
aimer completions fish --install
```

---

## Quick Start

### 1. Create a New Project

Run `aimer create` followed by your desired project name:

```bash
aimer create my_aimer_app
```

The interactive wizard will prompt you for:

- **App name**: Display name of your application, stored as `package.name` in `Aimer.toml` and used as the bundled app
  name (defaults to the project name)
- **Description**: Short description
- **Version**: Initial version (default: `0.1.0`)
- **Author**: Your name or team
- **Group**: Unique reverse domain identifier (e.g. `com.example.app`)
- **Targets**: Select which platform scaffolds to generate (`macos`, `windows`, `linux`, `android`, `ios`, `web`)

### 2. Project Layout

The generated project structure looks like this:

```text
my_aimer_app/
├── Aimer.toml           # Aimer framework manifest & asset declarations
├── Cargo.toml           # Rust package configuration
├── build.rs             # Build script with platform-specific link flags
├── README.md            # App documentation
├── src/
│   └── main.rs          # Application entry point & UI widget definitions
└── builds/              # Platform scaffolds and build output directories
    ├── macos/
    ├── ios/
    ├── android/
    ├── linux/
    ├── windows/
    └── web/
```

### 3. Run the Application

Enter your project directory:

```bash
cd my_aimer_app
```

Launch the interactive device picker:

```bash
aimer run
```

Or target a specific platform directly:

```bash
# macOS desktop
aimer run -t macos

# Web browser (launches local web server via Trunk)
aimer run -t web

# Android device or emulator
aimer run -t android

# iOS simulator
aimer run -t ios
```

---

## Command Reference

### `aimer create`

Scaffolds a new Aimer project with target configurations, templates, and platform build directories.

```bash
aimer create <PROJECT_NAME>
```

---

### `aimer run`

Builds, packages, and executes the project with an interactive developer console. By default the console is rendered
inline: finished stages scroll into your terminal history while the active stage and a hotkey bar stay at the bottom.
Use `--tui` for the full-screen console, or `--no-tui` for plain logs. When stdin or stdout is not a terminal (CI, an
IDE run window, a pipe), `aimer run` prints plain logs automatically.

```bash
aimer run [OPTIONS]
```

**Options:**

| Option                | Flag | Description                                                                                        |
|:----------------------|:-----|:---------------------------------------------------------------------------------------------------|
| `--target`            | `-t` | Specify the target platform directly (`macos`, `windows`, `linux`, `android`, `ios`, `web`)        |
| `--device`            | `-d` | Target a specific device ID without prompting                                                      |
| `--package`           | `-p` | Select an Aimer package in a Cargo workspace                                                       |
| `--release`           | `-r` | Build and package with release optimizations                                                       |
| `--tui`               |      | Use the full-screen TUI console instead of the default inline console (conflicts with `--no-tui`)  |
| `--no-tui`            |      | Disable the interactive console and stream plain logs to stdout/stderr (ideal for CI and IDEs)     |
| `--no-mcp`            |      | Disable the background private MCP control session                                                 |
| `-Z <FEATURE>`        |      | Enable nightly-only unstable features (`hot-reload`)                                               |
| `--verbose-widget-ir` |      | Print every Widget IR stage after native materialization (requires `-Z`)                           |
| `--verbose`           |      | Enable debug-level CLI logging                                                                     |

---

### `aimer build`

Compiles the Rust crate for the specified target without assembling a full platform package or launching the app.

```bash
aimer build [OPTIONS]
```

**Options:**

| Option      | Flag | Description                                                                           |
|:------------|:-----|:--------------------------------------------------------------------------------------|
| `--target`  | `-t` | Target platform to compile for (defaults to `[build].default_target` in `Aimer.toml`) |
| `--package` | `-p` | Select an Aimer package in a Cargo workspace                                        |
| `--release` | `-r` | Compile with release optimizations                                                    |

---

### `aimer assemble`

Assembles the distributable platform bundle (e.g. `.app`, `.apk`, `.ipa`, or static web directory).

```bash
aimer assemble [OPTIONS] <PLATFORM>
```

**Arguments:**

- `<PLATFORM>`: Target platform to bundle (`macos`, `windows`, `linux`, `android`, `ios`, `web`).

**Options:**

- `-r, --release`: Assemble in release mode for production distribution.
- `-p, --package <NAME>`: Select an Aimer package in a Cargo workspace.

Run, build, and assemble discover Aimer projects from `Aimer.toml` or `aimer.toml` files in the Cargo workspace. From
the workspace root, a single Aimer package is selected automatically. If there are multiple, use `-p` or `--package`.

**Examples:**

```bash
# Assemble release macOS .app bundle
aimer assemble macos --release

# Assemble release Android .apk
aimer assemble android --release

# Assemble Web distribution files
aimer assemble web --release

# Select one Aimer package in a workspace
aimer assemble android -p moewmoew
```

---

### `aimer doctor`

Diagnoses your development environment, reporting installed compilers, platform SDK tools, and cross-compilation target
triples.

```bash
aimer doctor
```

**What it verifies:**

- Rust compiler (`rustc`) & Cargo
- Web bundler (`trunk`)
- Xcode Command Line Tools (`xcrun`)
- Android SDK & Bridge (`adb`, `gradle`)
- Host linkers (`cc`, `pkg-config`, MSVC `link`)
- Required `rustup` target triples for all supported platforms, providing copy-paste commands for missing targets

---

### `aimer clean`

Removes build artifacts, generated files, intermediate libraries, and runs `cargo clean`.

```bash
aimer clean
```

Cleans:

- `builds/*/build/`
- `builds/android/app/build/` and `jniLibs/`
- `builds/web/pkg/` and `node_modules/`
- Cargo `target/` directory

---

### `aimer migrate`

Upgrades existing platform build scaffolds (`builds/<platform>/`) to the latest templates bundled with the current CLI
version while preserving your project settings and code.

```bash
aimer migrate <TARGET>
```

**Arguments:**

- `<TARGET>`: Platform to migrate (`macos`, `windows`, `linux`, `android`, `ios`, `web`, or `all`).

---

### `aimer completions`

Generates dynamic, self-updating shell completion scripts.

```bash
aimer completions <SHELL> [--install]
```

**Arguments:**

- `<SHELL>`: Shell type (`bash`, `zsh`, `fish`, `elvish`, `powershell`).

**Options:**

- `--install`: Writes the completion file into the conventional user completion directory and prints activation hints.

---

### `aimer mcp`

> Requires a build with the `mcp` feature (or `bundled-proc-macro` / `legacy-lsp`, which imply it). See [Installation](#installation).

Runs an MCP (Model Context Protocol) stdio server for Rust project tools and crate documentation. When the selected
project is an Aimer project with a live session, it also exposes session controls, logs, and resource monitoring. The
adapter discovers the nearest `Aimer.toml` or `Cargo.toml` from its working directory. Use `--attach` to select a
specific live Aimer session when the project has more than one.

```bash
# Install MCP into AI agents:
aimer mcp --install <claude|cursor|codex> [--global] [--project <PATH>]

# Run the MCP stdio adapter:
aimer mcp [--project <PATH>] [--attach <SESSION_ID>]
```

### `aimer skill`

Install or update Aimer's Rust workflow skill in the selected agent's user-level skills directory.

```bash
aimer skill --install <codex|claude|cursor> [<AGENT> ...]
```

For example, `aimer skill --install claude,codex` installs it for Claude Code and Codex. The command installs
`aimer-rust/SKILL.md`; its MCP tool list is generated from the server's `tools/list` response by
`scripts/update_aimer_rust_skill.py`.

---

### `aimer sessions`

Lists all live Aimer run sessions currently active on the machine.

```bash
aimer sessions [--project <PATH>] [--json]
```

**Options:**

- `--project <PATH>`: Restrict session discovery to a specific project directory.
- `--json`: Output session details in machine-readable JSON.

---

## Interactive Developer Console

When running `aimer run`, Aimer launches an interactive terminal interface (TUI) powered by Ratatui and Crossterm.

### Keyboard Shortcuts

| Shortcut               | Action               | Description                                          |
|:-----------------------|:---------------------|:-----------------------------------------------------|
| `r`                    | **Hot Reload**       | Triggers an in-place hot reload of application code  |
| `R` / `Shift + R`      | **Hot Restart**      | Stops the app, rebuilds, and relaunches fresh        |
| `q` / `Shift + Q`      | **Quit**             | Gracefully stops the running app and exits the CLI   |
| `1`                    | **App Pane**         | Focuses the Application log output pane              |
| `2`                    | **Build Pane**       | Focuses the Compiler / Build output pane             |
| `3`                    | **Inspector Pane**   | Focuses the Widget Inspector pane                    |
| `4`                    | **Resource Monitor** | Shows live CPU, GPU, memory, PID, and thread metrics  |
| `Tab`                  | **Next Pane**        | Cycles focus between available panes                 |
| `F12`                  | **Toggle Inspector** | Opens or hides the Widget Inspector view             |
| `t`                    | **Toggle Tree**      | Toggles the Widget Inspector full-tree view          |
| `e`                    | **Source Locations** | Toggles file & line number locations on log messages |
| `s`                    | **Selection Mode**   | Enables keyboard selection mode                      |
| `y`                    | **Yank**             | Copies the highlighted selection to clipboard        |
| `Shift + C`            | **Clear**            | Clears logs in the currently focused pane            |
| `Ctrl + C` / `Cmd + C` | **Copy**             | Copies the active selection or focused pane content  |
| `↑` / `↓`              | **Scroll**           | Scrolls the active pane up or down                   |
| `PgUp` / `PgDn`        | **Page Scroll**      | Scrolls active pane by full page                     |

---

## Model Context Protocol (MCP) Integration

Aimer features native integration with the **Model Context Protocol (MCP)**, allowing AI coding assistants to observe,
diagnose, and control your running application directly.

### Supported AI Agents

- **Claude Code** (`claude` / `claude-code`)
- **Cursor** (`cursor`)
- **Codex** (`codex`)

### Installing MCP Integration

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

### Available MCP Tools

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

### Resource monitoring

The TUI's Resource Monitor pane is available with `4` or `Tab`. Aimer samples the running app automatically and records each run under `~/.aimer/resource-recordings`; completed recordings are kept for 30 days by default and can be read later with the MCP tools above. The `record_resource_usage` tool temporarily applies the requested interval, waits for the duration, restores the previous interval, and returns that window's samples inline. It also returns the run recording ID for later paginated reads. The host collector uses `sysinfo` for process CPU and memory on macOS, Windows, and Linux. macOS GPU usage is collected from per-process Apple GPU records when available; Android process readings use the connected device through `adb`. iOS device process counters are unavailable to the host because of device sandboxing. Unavailable fields include a reason in the TUI and recorded data.

Resource settings live in the global `~/.aimer/config.toml`:

```toml
[resource_monitor]
sample_interval_ms = 1000 # 500–3000 ms
retention_days = 30       # 1–3650 days
```

---

## Project Configuration (`Aimer.toml`)

The `Aimer.toml` manifest file resides at the root of every Aimer project and defines application metadata, build
preferences, and asset bundling.

```toml
[package]
# Application name: the bundle is shipped under it (e.g. `My Aimer App.app`)
# and it is the label shown under the launcher icon. It may contain spaces.
# The Cargo package in `Cargo.toml` keeps the project name.
name = "My Aimer App"
version = "0.1.0"
description = "A cross-platform GUI app built with Aimer"
author = "Your Name <you@example.com>"
group = "com.example.app"

[build]
# Default platform used by `aimer build` when no --target is specified
default_target = "macos"

[assets]
# Files relative to the project root to bundle across all platforms
files = [
    "assets/icon.png",
    "assets/data.json",
]

# Optional hot-reload guest configuration (unstable / nightly)
[build.hot_reload]
package = "my_aimer_app"
program = "my_aimer_app::AppProgram"
limits = "my_aimer_app::APP_LIMITS"
```

---

## Assets Management

To bundle assets (images, fonts, JSON configuration) into your application across all supported targets:

1. Place your asset files inside your project directory (e.g. `assets/logo.png`).
2. Declare the paths under `[assets]` in `Aimer.toml`:

```toml
[assets]
files = [
    "assets/logo.png",
    "assets/fonts/Inter-Regular.ttf",
]
```

When building or assembling for any target (macOS, iOS, Android, Desktop, Web), `aimer` automatically copies these files
into the native platform asset directory (e.g., Android `assets/`, macOS `.app/Contents/Resources/`, iOS bundle
resources, web dist). The asset path declared in `Aimer.toml` serves as the runtime lookup key in your Rust code.

---

## Hot Reload & Unstable Features

Aimer features an experimental WebAssembly-based shadow hot reload pipeline that allows modifying widget code with
near-instant UI updates while preserving widget state.

To use unstable features, invoke `aimer` with the `+nightly` selector. The inline console is no longer one of them: it
is the default for `aimer run`, and the old `-Z inline-render` flag is still accepted (with a note) but does nothing.

```bash
# Run with WebAssembly hot-reload guest engine
aimer +nightly run -Z hot-reload

# Print detailed Widget Intermediate Representation (IR) stages
aimer +nightly run -Z hot-reload --verbose-widget-ir
```

---

## Workspace Architecture

The `aimer_cli` workspace is organized into modular crates:

```text
aimer_cli/
├── src/                      # Main `aimer` binary & library
│   ├── commands/             # CLI subcommand implementations
│   ├── console/              # Interactive TUI, Ratatui rendering, hotkey mappings
│   ├── session.rs            # Live session registry and IPC control server
│   └── targets.rs            # Target definitions and host architecture detection
├── aimer_hot_reload/         # Hot-reload pipeline, shadow guest generation, AST rewriting
├── aimer_reload_protocol/    # Cryptographic handshake, wire protocol, payload serialization
├── aimer_reload_server/      # Bridge server between the CLI runner and guest runtime
└── templates/                # Bundled project scaffolding and platform templates
```

- **`aimer_cli`**: The user-facing command-line interface, device manager, and orchestration engine.
- **`aimer_hot_reload`**: Manages source file watching, shadow package generation, and hot-reload guest compilation.
- **`aimer_reload_protocol`**: Implements authenticated, encrypted IPC channels between the CLI and the running
  application.
- **`aimer_reload_server`**: Embedded WebSocket/IPC communication server facilitating live reloads.

---

## Contributing & Development

Contributions to `aimer_cli` are welcome!

### Building and Testing

Check that the project compiles:

```bash
cargo check
```

Run the unit and integration test suite:

```bash
cargo test
```

Test running the CLI locally:

```bash
cargo run -- --help
cargo run -- doctor
```

---

## License

`aimer_cli` is part of the Aimer Framework project. Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

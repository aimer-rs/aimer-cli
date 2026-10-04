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

- **Rust**: Rust compiler (`rustc`) and package manager (`cargo`), version 2024 edition compatible (1.85+ recommended).
- **Rustup**: For managing cross-compilation targets.

Run `aimer doctor` after installation to verify platform dependencies and receive tailored setup commands.

---

## Installation

### From Source

Clone the repository and install the binary into your Cargo binary directory (`~/.cargo/bin`):

```bash
git clone https://github.com/Cottons29/aimer_cli.git
cd aimer_cli
cargo install --path .
```

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

- **App name**: Display name of your application
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

Builds, packages, and executes the project with an interactive developer console.

```bash
aimer run [OPTIONS]
```

**Options:**

| Option                | Flag | Description                                                                                        |
|:----------------------|:-----|:---------------------------------------------------------------------------------------------------|
| `--target`            | `-t` | Specify the target platform directly (`macos`, `windows`, `linux`, `android`, `ios`, `web`)        |
| `--device`            | `-d` | Target a specific device ID without prompting                                                      |
| `--release`           | `-r` | Build and package with release optimizations                                                       |
| `--no-tui`            |      | Disable the interactive terminal UI and stream plain logs to stdout/stderr (ideal for CI and IDEs) |
| `--no-mcp`            |      | Disable the background private MCP control session                                                 |
| `-Z <FEATURE>`        |      | Enable nightly-only unstable features (`hot-reload`, `inline-render`)                              |
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

**Examples:**

```bash
# Assemble release macOS .app bundle
aimer assemble macos --release

# Assemble release Android .apk
aimer assemble android --release

# Assemble Web distribution files
aimer assemble web --release
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

Runs an MCP (Model Context Protocol) stdio server for the current project's live session, resource history, and crate
documentation, or installs the MCP configuration into supported AI coding agents. Use `--attach` to select a specific
live session when the project has more than one.

```bash
# Install MCP into AI agents:
aimer mcp --install <claude|cursor|codex> [--global] [--project <PATH>]

# Run the MCP stdio adapter:
aimer mcp [--project <PATH>] [--attach <SESSION_ID>]
```

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
```

### Available MCP Tools

Once connected, your AI assistant can execute the following tools:

- `aimer_status`: Inspect the current run state, process ID, runtime, and target device.
- `aimer_restart_project`: Trigger a clean restart and rebuild of the app, waiting for the new run.
- `aimer_stop_project`: Stop the running project session.
- `aimer_get_logs`: Fetch bounded build, app, or system logs with cursor pagination and source code locations.
- `aimer_clear_logs`: Clear retained logs up to a specific sequence boundary.
- `aimer_get_resource_usage`: Read the latest app CPU, GPU, memory, PID, and thread sample.
- `aimer_set_resource_monitor_interval`: Set the live sample interval to 500–3000 ms; omit `interval_ms` to restore the global default.
- `aimer_record_resource_usage`: Capture a bounded sample window with `duration` in seconds (0.5–300) and `interval` in seconds (0.5–3), then return the samples inline and the run recording ID.
- `aimer_list_resource_recordings`: List retained per-run resource recordings, including completed app runs.
- `aimer_get_resource_recording`: Read paginated samples from a prior run.
- `aimer_get_crate_docs`: Read crate or item documentation from docs.rs. A trait lookup includes its rendered declaration, method signatures, required/provided status, and each method's documentation. The tool resolves an omitted version from the attached project's `Cargo.lock` and caches the crate's Rustdoc JSON index for offline lookups.
- `aimer_search_crate_docs`: Search public item names, paths, and documentation text within a crate. Results include item paths and short documentation summaries; pass a result's path to `aimer_get_crate_docs` to read the full item documentation.
- `aimer_rust_start_lsp`: Start or reuse rust-analyzer for `project_path`; wait for workspace indexing with progress notifications and return ready status. `project_path` may be an absolute path or a path relative to the current Rust project. Omitting it uses the current project.
- `aimer_rust_search_symbols`, `aimer_rust_search_text`, `aimer_rust_get_document_symbols`, and `aimer_rust_completion`: Search Rust workspace symbols and project text, inspect a file's symbol outline, or get context-aware completion suggestions.
- `aimer_rust_find_definition`, `aimer_rust_find_declaration`, `aimer_rust_type_definition`, `aimer_rust_find_references`, `aimer_rust_find_implementations`, `aimer_rust_hover`, `aimer_rust_get_call_hierarchy`, and `aimer_rust_expand_macro`: Navigate Rust code using rust-analyzer's semantic model.
- `aimer_rust_signature_help`, `aimer_rust_document_highlights`, `aimer_rust_inlay_hints`, and `aimer_rust_get_code_lenses`: Inspect call signatures, local symbol occurrences, inferred hints, and code lenses.
- `aimer_rust_get_diagnostics` and `aimer_rust_check`: Read current diagnostics or trigger rust-analyzer's Cargo flycheck and return structured diagnostics.
- `aimer_rust_get_code_actions`, `aimer_rust_prepare_rename`, `aimer_rust_rename_symbol`, `aimer_rust_preview_code_action`, `aimer_rust_format`, and `aimer_rust_apply_edit`: Prepare bounded edit previews and apply a preview after review.
- `aimer_rust_get_manifest` and `aimer_rust_get_project_metadata`: Read parsed `Cargo.toml` data or the selected workspace's bounded Cargo package and dependency metadata. Metadata uses `--locked` and does not update the project's lockfile.
- `aimer_rust_preview_manifest_edit`: Preview ordered `set` or `remove` operations on a `Cargo.toml` key path. The tool parses the candidate and runs Cargo metadata in a temporary workspace; invalid previews cannot be applied. Apply an applicable preview with `aimer_rust_apply_edit`.
- `aimer_rust_cargo_check` and `aimer_rust_cargo_test`: Run Cargo check or test for the selected workspace or a package and return the actual exit code with bounded stdout and stderr. Rustc warning diagnostics are filtered from the output and counted in `warnings_filtered`; other compiler diagnostics and test output remain visible. Both tools accept target and feature selection; test also accepts a test-name filter. `locked` defaults to Cargo's normal behavior, and the result reports whether `Cargo.lock` changed. Each call has a five-minute timeout.

Rust code tools require `rust-analyzer` on `PATH`. A custom binary and Cargo network policy can be set in `~/.aimer/config.toml`:

```toml
[mcp.rust_analyzer]
# Optional; defaults to finding rust-analyzer on PATH.
binary_path = "/path/to/rust-analyzer"
# Defaults to false. When false, Cargo calls made by Rust MCP tools run offline.
allow_network = false
```

The LSP process starts lazily when a Rust tool is first called, or explicitly through `aimer_rust_start_lsp`. The start tool waits for indexing and reports progress while rust-analyzer loads. The MCP server owns the rust-analyzer process and terminates it when the Agent disconnects and the MCP stdio process exits. Rust Analyzer enables build scripts and procedural macros for more complete analysis. `aimer_rust_check` triggers rust-analyzer's Clippy flycheck rather than starting Cargo directly; `aimer_rust_cargo_check` and `aimer_rust_cargo_test` run Cargo directly and return its exit code. Code edits are returned as previews and are only written after `aimer_rust_apply_edit`; stale previews are rejected. Manifest previews preserve existing TOML comments and formatting, and applying one asks rust-analyzer to reload the workspace. The rust-analyzer network setting applies to Cargo work launched by rust-analyzer and the new Cargo tools. Existing docs.rs tools retain their separate network and cache behavior.

The same docs.rs features are available directly from the CLI. It prints JSON, which makes the output easy to pipe into other tools:

```bash
# Read a crate's root documentation or a specific item
aimer docs serde
aimer docs serde de::Deserialize

# Search item names, paths, and documentation in a crate
aimer docs serde --search Deserialize --limit 10

# Select a version when it is not in the current project's Cargo.lock
aimer docs serde --search Deserialize --version 1.0.229
```

For trait results, the top-level `documentation` field contains the expanded trait description. `trait_details` contains `signature`, a Rust-like `definition`, and `methods` entries with each method's `name`, `signature`, `required` flag, and `documentation`.

The docs.rs cache is stored under `~/.aimer/docs/docs.rs`, with each crate version in its own subdirectory. It keeps the original gzip download and a zstd-compressed JSON copy for offline reads. Documentation is fetched on first use, so an uncached crate version requires an internet connection once.

### Resource monitoring

The TUI's Resource Monitor pane is available with `4` or `Tab`. Aimer samples the running app automatically and records each run under `~/.aimer/resource-recordings`; completed recordings are kept for 30 days by default and can be read later with the MCP tools above. The `aimer_record_resource_usage` tool temporarily applies the requested interval, waits for the duration, restores the previous interval, and returns that window's samples inline. It also returns the run recording ID for later paginated reads. The host collector uses `sysinfo` for process CPU and memory on macOS, Windows, and Linux. macOS GPU usage is collected from per-process Apple GPU records when available; Android process readings use the connected device through `adb`. iOS device process counters are unavailable to the host because of device sandboxing. Unavailable fields include a reason in the TUI and recorded data.

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
name = "my_aimer_app"
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

To use unstable features, invoke `aimer` with the `+nightly` selector:

```bash
# Run with WebAssembly hot-reload guest engine
aimer +nightly run -Z hot-reload

# Run with inline rendering
aimer +nightly run -Z inline-render

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

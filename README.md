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

`aimer` manages your Aimer project's full lifecycle from a single command: scaffolding, live development with hot
reload, native compilation, asset bundling, and release packaging across desktop, mobile, and web targets.

## Why `aimer`

- **One CLI, every platform.** `create`, `run`, `build`, and `assemble` work the same way for MacOS, Windoes and Linux,
  Linux, Windows, and Web.
- **An interactive developer console.** A Ratatui-powered TUI streams build output, app logs, widget inspection, and
  live resource usage, with hot reload and hot restart a keystroke away.
- **AI-agent ready.** A native [Model Context Protocol](docs/MCP.md) server lets Claude Code, Cursor, and Codex
  inspect runtime state, view logs, and control a running app directly.
- **Environment diagnostics built in.** `aimer doctor` checks installed toolchains, platform SDKs, and Rust
  cross-compilation targets, with copy-paste remediation commands.

## Quick Start

```bash
# Install from source
git clone https://github.com/Cottons29/aimer_cli.git
cd aimer_cli
cargo install --path .

# Scaffold and run a new app
aimer create my_aimer_app
cd my_aimer_app
aimer run
```

See [Installation](docs/INSTALLATION.md) for feature flags and shell completions, and
[Quick Start](docs/QUICK_START.md) for the full walkthrough and generated project layout.

## Supported Platforms

| Platform    | Target Identifier               | Build Artifact                 | Requirement                                               |
|:------------|:--------------------------------|:-------------------------------|:----------------------------------------------------------|
| **macOS**   | `macos`                         | `.app` bundle                  | macOS, Xcode & Command Line Tools                         |
| **iOS**     | `ios` / `ios-simulator`         | iOS App Bundle / Simulator App | macOS, Xcode, `xcrun`                                     |
| **Android** | `android` / `android-simulator` | Android `.apk`                 | Android SDK, `adb`, `cargo-ndk`, Gradle                   |
| **Linux**   | `linux`                         | Portable binary bundle         | GCC/Clang, `pkg-config`                                   |
| **Windows** | `windows`                       | Portable executable bundle     | MSVC C++ Build Tools (`link.exe`)                         |
| **Web**     | `web`                           | WebAssembly + HTML/JS          | [`trunk`](https://trunkrs.dev/), `wasm32-unknown-unknown` |

## Documentation

- [Installation](docs/INSTALLATION.md) — prerequisites, feature flags, and shell completions
- [Quick Start](docs/QUICK_START.md) — creating, running, and structuring a project
- [Command Reference](docs/COMMANDS.md) — every `aimer` subcommand and its options
- [Interactive Developer Console](docs/CONSOLE.md) — TUI keyboard shortcuts
- [Model Context Protocol (MCP) Integration](docs/MCP.md) — AI agent setup and the available tools
- [Project Configuration](docs/CONFIGURATION.md) — `Aimer.toml` and asset bundling
- [Hot Reload & Unstable Features](docs/HOT_RELOAD.md) — the nightly WASM hot-reload pipeline
- [Workspace Architecture](docs/ARCHITECTURE.md) — crate layout and responsibilities
- [Contributing & Development](docs/CONTRIBUTING.md) — building, testing, and CI

## License

`aimer_cli` is part of the Aimer Framework project. Licensed under either of:

- Apache License, Version 2.0 (http://www.apache.org/licenses/LICENSE-2.0)
- MIT license (http://opensource.org/licenses/MIT)

at your option.


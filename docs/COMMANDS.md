[← Back to README](../README.md)

# Command Reference

- [`aimer create`](#aimer-create)
- [`aimer init`](#aimer-init)
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

---

## `aimer create`

Scaffolds a new Aimer project with target configurations, templates, and platform build directories.

```bash
aimer create <PROJECT_NAME>
```

---

## `aimer init`

Turns an existing Cargo project into an Aimer project. It asks the same questions as `aimer create` and generates the
same `Aimer.toml` and `builds/<platform>/` scaffolds, but works in place: your sources and README are left alone, and
`Cargo.toml` (an Aimer-capable `[lib]` and the `aimer` dependency) and `.gitignore` are only extended.
It refuses to run on a project that already has an `Aimer.toml` (use `aimer migrate`) or to replace an existing
`builds/<platform>/` folder.

```bash
aimer init [PATH]
```

**Arguments:**

- `[PATH]`: Cargo project directory (defaults to the current directory).

Your entry point still has to start the app, as in the `aimer create` template (`#[aimer::main]` and
`AimerApp::start(...)`).

---

## `aimer run`

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

See [Interactive Developer Console](CONSOLE.md) for keyboard shortcuts, and
[Hot Reload & Unstable Features](HOT_RELOAD.md) for `-Z hot-reload`.

---

## `aimer build`

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

## `aimer assemble`

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

## `aimer doctor`

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

## `aimer clean`

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

## `aimer migrate`

Upgrades existing platform build scaffolds (`builds/<platform>/`) to the latest templates bundled with the current CLI
version while preserving your project settings and code.

```bash
aimer migrate <TARGET>
```

**Arguments:**

- `<TARGET>`: Platform to migrate (`macos`, `windows`, `linux`, `android`, `ios`, `web`, or `all`).

---

## `aimer completions`

Generates dynamic, self-updating shell completion scripts.

```bash
aimer completions <SHELL> [--install]
```

**Arguments:**

- `<SHELL>`: Shell type (`bash`, `zsh`, `fish`, `elvish`, `powershell`).

**Options:**

- `--install`: Writes the completion file into the conventional user completion directory and prints activation hints.

---

## `aimer mcp`

> Requires a build with the `mcp` feature (or `bundled-proc-macro` / `legacy-lsp`, which imply it). See [Installation](INSTALLATION.md).

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

See [Model Context Protocol (MCP) Integration](MCP.md) for supported agents and the full tool list.

---

## `aimer skill`

Install or update Aimer's Rust workflow skill in the selected agent's user-level skills directory.

```bash
aimer skill --install <codex|claude|cursor> [<AGENT> ...]
```

For example, `aimer skill --install claude,codex` installs it for Claude Code and Codex. The command installs
`aimer-rust/SKILL.md`; its MCP tool list is generated from the server's `tools/list` response by
`scripts/update_aimer_rust_skill.py`.

---

## `aimer sessions`

Lists all live Aimer run sessions currently active on the machine.

```bash
aimer sessions [--project <PATH>] [--json]
```

**Options:**

- `--project <PATH>`: Restrict session discovery to a specific project directory.
- `--json`: Output session details in machine-readable JSON.

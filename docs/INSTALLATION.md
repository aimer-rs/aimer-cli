[← Back to README](../README.md)

# Installation

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

## From Source

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

---

## Enable Shell Completions

Generate and install completions for your active shell:

```bash
# For Zsh
aimer completions zsh --install

# For Bash
aimer completions bash --install

# For Fish
aimer completions fish --install
```

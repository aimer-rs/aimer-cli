[← Back to README](../README.md)

# Contributing & Development

Contributions to `aimer_cli` are welcome!

## Building and Testing

Check that the project compiles:

```bash
cargo check
```

Run the unit and integration test suite:

```bash
cargo test
```

Or with [`cargo-nextest`](https://nexte.st/) (used by CI, see below):

```bash
cargo nextest run --workspace
```

Test running the CLI locally:

```bash
cargo run -- --help
cargo run -- doctor
```

## Continuous Integration

The [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) workflow checks and runs the `cargo-nextest` test suite
on every push and pull request across five targets: `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`,
`aarch64-pc-windows-msvc`, `aarch64-unknown-linux-gnu`, and `aarch64-apple-darwin`. Each target runs independently
(`fail-fast: false`), and `cargo nextest run` is invoked with `--no-fail-fast` so a single failing test does not hide
the rest — every failure across every target is reported instead of stopping at the first one.

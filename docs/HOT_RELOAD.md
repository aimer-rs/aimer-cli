[← Back to README](../README.md)

# Hot Reload & Unstable Features

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

See [Project Configuration](CONFIGURATION.md) for the `[build.hot_reload]` settings in `Aimer.toml`, and
[Workspace Architecture](ARCHITECTURE.md) for the crates that implement the pipeline.

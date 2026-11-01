[← Back to README](../README.md)

# Workspace Architecture

The `aimer_cli` workspace is organized into modular crates:

```text
aimer_cli/
├── src/                      # Main `aimer` binary & library
│   ├── commands/             # CLI subcommand implementations
│   ├── console/              # Interactive TUI, Ratatui rendering, hotkey mappings
│   ├── session.rs            # Live session registry and IPC control server
│   └── targets.rs            # Target definitions and host architecture detection
├── aimer_config/             # `Aimer.toml` manifest, signing metadata, and user configuration
└── templates/                # Bundled project scaffolding and platform templates
```

- **`aimer_cli`**: The user-facing command-line interface, device manager, and orchestration engine.
- **`aimer_config`**: Parses the `Aimer.toml` project manifest, per-platform signing metadata, and the user-level
  configuration for MCP and the resource monitor.

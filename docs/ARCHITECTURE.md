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

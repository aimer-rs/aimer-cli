---
name: aimer-rust
description: Use Aimer MCP's Rust LSP tools for Rust code search, navigation, diagnostics, and preview-based edits in the selected Rust workspace. Prefer Aimer for semantic operations when available; use Codex file tools to inspect source and as a fallback.
---

# Aimer Rust

Use this workflow for Rust tasks in any workspace or crates selected in Aimer MCP.

## Start with the requested task

- Do not scan or index the workspace just because a Codex session started. Use the narrowest search or LSP operation that answers the current Rust task.
- Aimer Rust LSP tools start or reuse rust-analyzer and wait for indexing. Keep the call pending while progress is reported; do not treat an unfinished call as an empty result.
- Sometime the requirements tools is not install inside the SandBox, please install rust-analyzer and clippy with rustup manually.
- If the user asks to work in a different workspace, select it with `aimer_rust_start_lsp` and its `project_path`. This changes the selected workspace for later Aimer Rust calls.

## Search and inspect

- Use `search_symbols` for symbol names or paths. It ranks exact matches ahead of fuzzy matches; add `kind` and a focused `limit` when useful.
- Use `search_text` for literal text, comments, macro bodies, or other text that is not a symbol lookup. Narrow it with `file_glob`, case sensitivity, and `limit` as needed.
- Use `inspect_file` for selected source ranges and focused matches. Each range returns one compact `text` block with `start_line`; count lines from that value when reporting positions, and check `truncated_lines` before relying on long lines.
- Use `get_document_symbols` for a file outline. For implementation details, read the relevant source with Codex file tools; symbol results and hover are not a substitute for inspecting code bodies.
- Use the position-based LSP tools for definitions, declarations, type definitions, references, implementations, hover, signature help, call hierarchy, highlights, inlay hints, and macro expansion. Their lines and columns are 1-based, with columns measured in UTF-16 code units. Derive positions from source or returned locations; do not guess.
- If hover has no documentation, inspect the source doc comments. For dependency documentation, use Aimer's crate documentation tool when available.
- Summarize only the relevant result fields. Do not repeat a large JSON response unless the user asks for the raw result.

## Make Rust edits safely

- Use Codex editing tools for ordinary code changes. Use Aimer's LSP edit tools when semantic rename, a specific code action, or formatting is the right operation.
- Treat rename, code-action, format, and `preview_edit` results as previews. Inspect each proposed diff and its validation report before applying it with `apply_edit`. If a preview is stale, create a fresh one.
- Use `preview_edit` for precise local edits or project file creation/deletion. Supply project-relative paths and exact `old_text`/`new_text` replacements; each old anchor must match exactly once, and replacements in one file must not overlap. Do not use it for `Cargo.toml`; use `preview_manifest_edit` so Cargo validates the candidate.
- For rename results, surface possible unedited identifiers in `macro_rules!` bodies. Aimer does not add those candidates automatically; do not silently replace them or claim they were renamed.
- Do not apply a code action or formatting change unless it is part of the user's requested work.

## Diagnostics and checks

- Use `get_diagnostics` for diagnostics already known to rust-analyzer.
- `check` runs Clippy with all warnings denied. Use it when the user requests a check or the task requires that verification. Clippy must be installed; Aimer reports diagnostics but rust-analyzer does not expose Cargo's exit code. Cargo runs offline unless the user's Aimer configuration enables network access.

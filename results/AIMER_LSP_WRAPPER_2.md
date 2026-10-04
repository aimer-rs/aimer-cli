# Aimer Rust LSP Wrapper — Bamboo Test Report 2

**Date:** 2026-10-02  
**Project tested:** `/Users/cottons/Documents/bamboo`  
**MCP command:** `/Users/cottons/AimerFramework/AimerCli/target/debug/aimer mcp --project /Users/cottons/Documents/bamboo`  
**Transport:** MCP stdio / JSON-RPC 2.0  
**Binary build:** `cargo build --offline --bin aimer` succeeded before the run.

## Outcome

- MCP advertised **24 Rust LSP tools**; the harness completed **25 calls** (2 exact/fuzzy symbol-search queries).
- All 24 advertised tools called: yes.
- Bamboo Git status before: `clean`; after: `clean`.
- Format, rename, and code-action calls returned previews only. `aimer_rust_apply_edit` received an intentionally invalid preview ID and rejected it; no Bamboo edit was applied.
- The main macro-expansion probe uses a normal `vec!` invocation; type-definition targets the `Pubspec` return type.
- This report retains complete MCP tool-call request/response envelopes and all associated progress notifications.

## Readiness and environment

The first search waited for rust-analyzer readiness and returned only after progress notifications indicated stable workspace state. RA reported `health=warning`: Cargo could not open `/Users/cottons/Documents/bamboo/target/debug/.cargo-build-lock` (`Operation not permitted`), so `aimer_rust_check` returned an empty diagnostics list but does **not** verify a successful Cargo check in this sandbox. Semantic LSP requests still returned results. The complete raw warning is retained in the notifications below.

## Summary by tool call

| # | Tool | Outcome | Time | Progress events |
|---:|---|---|---:|---:|
| 1 | `aimer_rust_search_symbols` | results=1 | 3.729s | 6 |
| 2 | `aimer_rust_search_symbols` | results=1 | 0.254s | 0 |
| 3 | `aimer_rust_search_text` | results=8 (truncated) | 0.018s | 0 |
| 4 | `aimer_rust_get_document_symbols` | results=2 | 3.282s | 3 |
| 5 | `aimer_rust_check` | diagnostics=0 (Cargo watcher failed; check unverified) | 1.069s | 0 |
| 6 | `aimer_rust_get_diagnostics` | diagnostics=0 | 0.509s | 0 |
| 7 | `aimer_rust_completion` | results=16 | 0.794s | 0 |
| 8 | `aimer_rust_document_highlights` | results=1 | 0.506s | 0 |
| 9 | `aimer_rust_expand_macro` | returned structured JSON | 3.312s | 3 |
| 10 | `aimer_rust_find_declaration` | results=1 | 3.304s | 3 |
| 11 | `aimer_rust_find_definition` | results=1 | 0.507s | 0 |
| 12 | `aimer_rust_find_implementations` | results=200 (truncated) | 3.579s | 3 |
| 13 | `aimer_rust_find_references` | results=15 | 0.622s | 0 |
| 14 | `aimer_rust_format` | preview changed=true, files=1 | 0.567s | 0 |
| 15 | `aimer_rust_get_call_hierarchy` | results=13 | 0.765s | 0 |
| 16 | `aimer_rust_get_code_actions` | results=2 | 0.807s | 0 |
| 17 | `aimer_rust_get_code_lenses` | results=0 | 0.505s | 0 |
| 18 | `aimer_rust_hover` | hover type and documentation returned | 0.516s | 0 |
| 19 | `aimer_rust_inlay_hints` | results=5 | 0.550s | 0 |
| 20 | `aimer_rust_prepare_rename` | rename target range returned | 0.507s | 0 |
| 21 | `aimer_rust_rename_symbol` | preview changed=true, files=9 | 0.594s | 0 |
| 22 | `aimer_rust_preview_code_action` | preview changed=true, files=1 | 0.002s | 0 |
| 23 | `aimer_rust_signature_help` | signatures=1 | 0.506s | 0 |
| 24 | `aimer_rust_type_definition` | results=1 | 0.508s | 0 |
| 25 | `aimer_rust_apply_edit` | expected rejection of invalid preview ID | 0.004s | 0 |

## MCP initialize response

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "tools": {}
    },
    "serverInfo": {
      "name": "rmcp",
      "version": "3.5.0"
    },
    "instructions": "Rust LSP tools wait for rust-analyzer to finish loading and indexing the selected workspace before returning results. Keep an LSP tool call pending while it reports indexing progress; do not interpret an unfinished call as an empty search result. Readiness timeouts and rust-analyzer health errors are returned explicitly."
  }
}
```

## Advertised Rust LSP tools

- `aimer_rust_apply_edit` — Apply a previously reviewed Rust edit preview if its source files are unchanged.
- `aimer_rust_check` — Trigger rust-analyzer's Cargo flycheck and wait for its structured diagnostics. Cargo runs offline unless user config opts in to network access.
- `aimer_rust_completion` — Return bounded Rust completion suggestions at a project-relative source position.
- `aimer_rust_document_highlights` — Find symbol occurrences highlighted within the current Rust document.
- `aimer_rust_expand_macro` — Expand the Rust macro invocation at a project-relative file position.
- `aimer_rust_find_declaration` — Find the declaration of the Rust symbol at a project-relative file position.
- `aimer_rust_find_definition` — Find the definition of the Rust symbol at a project-relative file position.
- `aimer_rust_find_implementations` — Find Rust implementations for the symbol at a project-relative file position.
- `aimer_rust_find_references` — Find Rust symbol references at a project-relative file position.
- `aimer_rust_format` — Format a Rust file or range and return a preview; apply it with aimer_rust_apply_edit.
- `aimer_rust_get_call_hierarchy` — Return callers or callees for the function at a project-relative file position.
- `aimer_rust_get_code_actions` — List code actions available for a project-relative source range.
- `aimer_rust_get_code_lenses` — Return bounded code lenses for a Rust file, resolving their command metadata without executing commands.
- `aimer_rust_get_diagnostics` — Return the latest rust-analyzer diagnostics as bounded structured JSON.
- `aimer_rust_get_document_symbols` — Return the nested Rust symbol outline for a project file.
- `aimer_rust_hover` — Return the Rust type and documentation information at a project-relative file position.
- `aimer_rust_inlay_hints` — Return inferred type and parameter-name hints for a Rust source range.
- `aimer_rust_prepare_rename` — Check whether the Rust symbol at a source position can be renamed and return its target range and placeholder.
- `aimer_rust_preview_code_action` — Resolve a code action and return a preview; apply it with aimer_rust_apply_edit.
- `aimer_rust_rename_symbol` — Prepare a semantic symbol rename and return a preview. Possible unedited identifiers in macro_rules! bodies are listed separately and are never added automatically; apply the preview with aimer_rust_apply_edit.
- `aimer_rust_search_symbols` — Fuzzy-search Rust workspace symbols. Case-insensitive exact name or path matches come first, followed by fuzzy matches; each result includes match_quality. Returns bounded JSON locations.
- `aimer_rust_search_text` — Search project text, comments, and string literals; respects Git ignore rules when available and returns bounded JSON matches.
- `aimer_rust_signature_help` — Return function signature and active parameter information at a Rust source position.
- `aimer_rust_type_definition` — Find the type definition of the Rust symbol at a project-relative file position.

## Raw MCP tool calls

Each section includes arguments, the JSON-RPC request, any progress notifications, and the complete MCP response. `TextContent.text` is preserved verbatim.

### 1. `aimer_rust_search_symbols`

**Purpose:** Exact symbol-name query.  
**Elapsed:** 3.729s  
**Observed:** results=1

**Arguments**

````json
{
  "query": "read_pubspec_yaml",
  "limit": 12
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_search_symbols",
    "arguments": {
      "query": "read_pubspec_yaml",
      "limit": 12
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-3"
    }
  }
}
````

**Raw progress notifications**

````json
[
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "Starting rust-analyzer; this tool will wait for workspace indexing to finish.",
      "progress": 1.0,
      "progressToken": "aimer-lsp-report-3"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer is loading and indexing the workspace; wait for this tool to finish.",
      "progress": 2.0,
      "progressToken": "aimer-lsp-report-3"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer is still indexing the workspace (health=ok): no additional status",
      "progress": 3.0,
      "progressToken": "aimer-lsp-report-3"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer is still indexing the workspace (health=warning): Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.",
      "progress": 4.0,
      "progressToken": "aimer-lsp-report-3"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer reports no pending workspace work; confirming stable readiness (health=warning): Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.",
      "progress": 5.0,
      "progressToken": "aimer-lsp-report-3"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer workspace is ready with a warning: Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.",
      "progress": 6.0,
      "progressToken": "aimer-lsp-report-3"
    }
  }
]
````

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"containerName\":\"PubspecFile\",\"kind\":\"function\",\"location\":{\"file\":\"bamboo_core/src/file_reader.rs\",\"range\":{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}}},\"match_quality\":\"exact\",\"name\":\"read_pubspec_yaml\"}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 2. `aimer_rust_search_symbols`

**Purpose:** Near-spelling fuzzy symbol query.  
**Elapsed:** 0.254s  
**Observed:** results=1

**Arguments**

````json
{
  "query": "read_pubspec_yml",
  "limit": 12
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_search_symbols",
    "arguments": {
      "query": "read_pubspec_yml",
      "limit": 12
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-4"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"containerName\":\"PubspecFile\",\"kind\":\"function\",\"location\":{\"file\":\"bamboo_core/src/file_reader.rs\",\"range\":{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}}},\"match_quality\":\"fuzzy\",\"name\":\"read_pubspec_yaml\"}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 3. `aimer_rust_search_text`

**Purpose:** Literal workspace text search.  
**Elapsed:** 0.018s  
**Observed:** results=8 (truncated)

**Arguments**

````json
{
  "query": "read_pubspec_yaml",
  "file_glob": "**/*.rs",
  "limit": 8
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_search_text",
    "arguments": {
      "query": "read_pubspec_yaml",
      "file_glob": "**/*.rs",
      "limit": 8
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-5"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 5,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"query\":\"read_pubspec_yaml\",\"results\":[{\"column\":27,\"file\":\"bamboo_core/src/bamboo_util.rs\",\"line\":24,\"text\":\"Err(_) => PubspecFile::read_pubspec_yaml(current_dir)?,\"},{\"column\":29,\"file\":\"bamboo_core/src/bamboo_util.rs\",\"line\":264,\"text\":\"Err(_) => PubspecFile::read_pubspec_yaml(current_dir)\"},{\"column\":41,\"file\":\"bamboo_core/src/create_cmd/cmd_create.rs\",\"line\":193,\"text\":\"let mut pubspec_object = PubspecFile::read_pubspec_yaml(&yaml_path)?;\"},{\"column\":26,\"file\":\"bamboo_core/src/create_cmd/cmd_create.rs\",\"line\":226,\"text\":\"PubspecFile::read_pubspec_yaml(self.parent_dir.as_ref().unwrap())\"},{\"column\":12,\"file\":\"bamboo_core/src/file_reader.rs\",\"line\":25,\"text\":\"pub fn read_pubspec_yaml(dir: impl AsRef<Path>) -> io::Result<Pubspec> {\"},{\"column\":29,\"file\":\"bamboo_core/src/package_cmd/cmd_build.rs\",\"line\":60,\"text\":\"Err(_) => PubspecFile::read_pubspec_yaml(&self.current_dir)?,\"},{\"column\":29,\"file\":\"bamboo_core/src/package_cmd/cmd_build.rs\",\"line\":75,\"text\":\"Err(_) => PubspecFile::read_pubspec_yaml(&self.current_dir)?,\"},{\"column\":34,\"file\":\"bamboo_core/src/package_cmd/cmd_init.rs\",\"line\":70,\"text\":\"PubspecFile::read_pubspec_yaml(&self.current_dir)?\"}],\"truncated\":true}"
      }
    ],
    "isError": false
  }
}
````

### 4. `aimer_rust_get_document_symbols`

**Purpose:** Document-symbol outline.  
**Elapsed:** 3.282s  
**Observed:** results=2

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs"
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_get_document_symbols",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs"
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-6"
    }
  }
}
````

**Raw progress notifications**

````json
[
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer is loading and indexing the workspace; wait for this tool to finish.",
      "progress": 1.0,
      "progressToken": "aimer-lsp-report-6"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer reports no pending workspace work; confirming stable readiness (health=warning): Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.",
      "progress": 2.0,
      "progressToken": "aimer-lsp-report-6"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer workspace is ready with a warning: Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.",
      "progress": 3.0,
      "progressToken": "aimer-lsp-report-6"
    }
  }
]
````

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 6,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"deprecated\":false,\"kind\":\"struct\",\"name\":\"PubspecFile\",\"range\":{\"end\":{\"column\":24,\"line\":14},\"start\":{\"column\":1,\"line\":14}},\"selection_range\":{\"end\":{\"column\":23,\"line\":14},\"start\":{\"column\":12,\"line\":14}},\"tags\":[]},{\"children\":[{\"deprecated\":false,\"detail\":\"fn(dir: impl AsRef<Path>) -> PathBuf\",\"kind\":\"function\",\"name\":\"pubspec_path\",\"range\":{\"end\":{\"column\":6,\"line\":19},\"start\":{\"column\":5,\"line\":17}},\"selection_range\":{\"end\":{\"column\":24,\"line\":17},\"start\":{\"column\":12,\"line\":17}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(dir: impl AsRef<Path>) -> io::Result<Pubspec>\",\"kind\":\"function\",\"name\":\"read_pubspec_yaml\",\"range\":{\"end\":{\"column\":6,\"line\":28},\"start\":{\"column\":5,\"line\":21}},\"selection_range\":{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(value: &mut serde_yaml::Value)\",\"kind\":\"function\",\"name\":\"remove_null_fields\",\"range\":{\"end\":{\"column\":6,\"line\":55},\"start\":{\"column\":5,\"line\":30}},\"selection_range\":{\"end\":{\"column\":26,\"line\":32},\"start\":{\"column\":8,\"line\":32}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(value: &serde_yaml::Value) -> HashSet<String>\",\"kind\":\"function\",\"name\":\"top_level_sequence_keys\",\"range\":{\"end\":{\"column\":6,\"line\":70},\"start\":{\"column\":5,\"line\":57}},\"selection_range\":{\"end\":{\"column\":31,\"line\":58},\"start\":{\"column\":8,\"line\":58}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(value: &serde_yaml::Value) -> HashSet<String>\",\"kind\":\"function\",\"name\":\"top_level_scalar_sequence_keys\",\"range\":{\"end\":{\"column\":6,\"line\":93},\"start\":{\"column\":5,\"line\":72}},\"selection_range\":{\"end\":{\"column\":38,\"line\":73},\"start\":{\"column\":8,\"line\":73}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(yaml: &str, sequence_keys: &HashSet<String>) -> String\",\"kind\":\"function\",\"name\":\"add_blank_lines_before_vec_fields\",\"range\":{\"end\":{\"column\":6,\"line\":127},\"start\":{\"column\":5,\"line\":95}},\"selection_range\":{\"end\":{\"column\":41,\"line\":97},\"start\":{\"column\":8,\"line\":97}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(yaml: &str, keys: &HashSet<String>) -> String\",\"kind\":\"function\",\"name\":\"add_indent_for_scalar_seq_fields\",\"range\":{\"end\":{\"column\":6,\"line\":168},\"start\":{\"column\":5,\"line\":129}},\"selection_range\":{\"end\":{\"column\":40,\"line\":131},\"start\":{\"column\":8,\"line\":131}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(pubspec: &Pubspec) -> Result<serde_yaml::Value, Box<dyn Error>>\",\"kind\":\"function\",\"name\":\"build_ordered_pubspec_value\",\"range\":{\"end\":{\"column\":6,\"line\":261},\"start\":{\"column\":5,\"line\":170}},\"selection_range\":{\"end\":{\"column\":35,\"line\":172},\"start\":{\"column\":8,\"line\":172}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn( dir: impl AsRef<Path>, pubspec: &Pubspec, ) -> Result<(), Box<dyn Error>>\",\"kind\":\"function\",\"name\":\"write_pubspec_yaml\",\"range\":{\"end\":{\"column\":6,\"line\":331},\"start\":{\"column\":5,\"line\":263}},\"selection_range\":{\"end\":{\"column\":30,\"line\":270},\"start\":{\"column\":12,\"line\":270}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(dir: impl AsRef<Path>) -> PathBuf\",\"kind\":\"function\",\"name\":\"pubspec_toml_path\",\"range\":{\"end\":{\"column\":6,\"line\":336},\"start\":{\"column\":5,\"line\":333}},\"selection_range\":{\"end\":{\"column\":29,\"line\":334},\"start\":{\"column\":12,\"line\":334}},\"tags\":[]},{\"deprecated\":false,\"detail\":\"fn(dir: impl AsRef<Path>) -> io::Result<Pubspec>\",\"kind\":\"function\",\"name\":\"read_pubspec_toml\",\"range\":{\"end\":{\"column\":6,\"line\":362},\"start\":{\"column\":5,\"line\":338}},\"selection_range\":{\"end\":{\"column\":29,\"line\":345},\"start\":{\"column\":12,\"line\":345}},\"tags\":[]},{\"children\":[{\"children\":[{\"deprecated\":false,\"detail\":\"fn(item: &Item) -> Option<Value>\",\"kind\":\"function\",\"name\":\"item_to_value_inline\",\"range\":{\"end\":{\"column\":14,\"line\":486},\"start\":{\"column\":13,\"line\":471}},\"selection_range\":{\"end\":{\"column\":36,\"line\":472},\"start\":{\"column\":16,\"line\":472}},\"tags\":[]}],\"deprecated\":false,\"detail\":\"fn(table_item: &mut Item)\",\"kind\":\"function\",\"name\":\"inline_children\",\"range\":{\"end\":{\"column\":10,\"line\":507},\"start\":{\"column\":9,\"line\":469}},\"selection_range\":{\"end\":{\"column\":27,\"line\":470},\"start\":{\"column\":12,\"line\":470}},\"tags\":[]}],\"deprecated\":false,\"detail\":\"fn( dir: impl AsRef<Path>, pubspec: &Pubspec, ) -> Result<(), Box<dyn Error>>\",\"kind\":\"function\",\"name\":\"write_pubspec_toml\",\"range\":{\"end\":{\"column\":6,\"line\":542},\"start\":{\"column\":5,\"line\":364}},\"selection_range\":{\"end\":{\"column\":30,\"line\":372},\"start\":{\"column\":12,\"line\":372}},\"tags\":[]}],\"deprecated\":false,\"kind\":\"object\",\"name\":\"impl PubspecFile\",\"range\":{\"end\":{\"column\":2,\"line\":543},\"start\":{\"column\":1,\"line\":16}},\"selection_range\":{\"end\":{\"column\":17,\"line\":16},\"start\":{\"column\":6,\"line\":16}},\"tags\":[]}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 5. `aimer_rust_check`

**Purpose:** Trigger rust-analyzer Cargo flycheck.  
**Elapsed:** 1.069s  
**Observed:** diagnostics=0 (Cargo watcher failed; check unverified)

**Arguments**

````json
{}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_check",
    "arguments": {},
    "_meta": {
      "progressToken": "aimer-lsp-report-7"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 7,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"diagnostics\":[],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 6. `aimer_rust_get_diagnostics`

**Purpose:** Read diagnostics for the test file.  
**Elapsed:** 0.509s  
**Observed:** diagnostics=0

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs"
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 8,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_get_diagnostics",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs"
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-8"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 8,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"diagnostics\":[],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 7. `aimer_rust_completion`

**Purpose:** Completion at Self:: member access.  
**Elapsed:** 0.794s  
**Observed:** results=16

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "line": 26,
  "column": 48
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 9,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_completion",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "line": 26,
      "column": 48
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-9"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 9,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"is_incomplete\":true,\"results\":[{\"detail\":\"fn(impl AsRef<Path>) -> PathBuf\",\"filterText\":\"pubspec_path\",\"kind\":\"function\",\"label\":\"pubspec_path\",\"preselect\":true,\"sortText\":\"7fffffd3\",\"textEdit\":{\"newText\":\"pubspec_path\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(impl AsRef<Path>) -> Result<Pubspec, Error>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Reads the `pubspec.yaml` file from the given directory and deserializes it into `PubspecObject`.\\n\\n# Errors\\nReturns an `io::Error` if the file cannot be read or if the YAML is invalid.\"},\"filterText\":\"read_pubspec_yaml\",\"kind\":\"function\",\"label\":\"read_pubspec_yaml\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"read_pubspec_yaml\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(&mut Value)\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Remove all null-valued entries from mappings inside a YAML value.\\nThis effectively drops fields that would have been serialized from `Option::None`.\"},\"filterText\":\"remove_null_fields\",\"kind\":\"function\",\"label\":\"remove_null_fields\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"remove_null_fields\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(&Value) -> HashSet<String, RandomState, Global>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Collects the top-level keys whose values are sequences (Vec in Rust terms).\"},\"filterText\":\"top_level_sequence_keys\",\"kind\":\"function\",\"label\":\"top_level_sequence_keys\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"top_level_sequence_keys\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(&Value) -> HashSet<String, RandomState, Global>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Collects the top-level keys whose values are sequences of scalars (no nested maps/sequences).\"},\"filterText\":\"top_level_scalar_sequence_keys\",\"kind\":\"function\",\"label\":\"top_level_scalar_sequence_keys\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"top_level_scalar_sequence_keys\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(&str, &HashSet<String, RandomState, Global>) -> String\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Insert a blank line before any top-level mapping entry whose value is a sequence.\\nThis operates on the YAML text assuming standard formatting from `serde_yaml::to_string`.\"},\"filterText\":\"add_blank_lines_before_vec_fields\",\"kind\":\"function\",\"label\":\"add_blank_lines_before_vec_fields\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"add_blank_lines_before_vec_fields\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(&str, &HashSet<String, RandomState, Global>) -> String\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Indent sequence items (lines starting with \\\"- \\\") by two spaces for specified top-level keys\\nwhose values are sequences of scalars (e.g., `workspace`).\"},\"filterText\":\"add_indent_for_scalar_seq_fields\",\"kind\":\"function\",\"label\":\"add_indent_for_scalar_seq_fields\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"add_indent_for_scalar_seq_fields\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(&Pubspec) -> Result<Value, Box<dyn Error + 'static, Global>>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Build a YAML mapping with fields ordered per Dart pubspec conventions.\\nOnly includes fields that are `Some(...)`. Extra fields are appended at the end (sorted).\"},\"filterText\":\"build_ordered_pubspec_value\",\"kind\":\"function\",\"label\":\"build_ordered_pubspec_value\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"build_ordered_pubspec_value\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(impl AsRef<Path>, &Pubspec) -> Result<(), Box<dyn Error + 'static, Global>>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Serializes a `PubspecObject` and writes it to `pubspec.yaml` in the given directory,\\nordered per Dart pubspec, only including fields that are `Some(...)`, and\\ninserting a blank line before any top-level field whose value is a sequence.\\nAdditionally, indent sequence items for scalar sequences (e.g., `workspace`) by two spaces.\\n\\n# Errors\\nReturns an `io::Error` if serialization fails or the file cannot be written.\"},\"filterText\":\"write_pubspec_yaml\",\"kind\":\"function\",\"label\":\"write_pubspec_yaml\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"write_pubspec_yaml\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(impl AsRef<Path>) -> PathBuf\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Returns the path to `pubspec.toml` inside the provided directory.\"},\"filterText\":\"pubspec_toml_path\",\"kind\":\"function\",\"label\":\"pubspec_toml_path\",\"preselect\":true,\"sortText\":\"7fffffd3\",\"textEdit\":{\"newText\":\"pubspec_toml_path\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(impl AsRef<Path>) -> Result<Pubspec, Error>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Reads `pubspec.toml` from the given directory and deserializes it into `PubspecObject`.\\nSupports two shapes for workspace:\\n- Legacy: `workspace = [\\\"a\\\", \\\"b\\\"]`\\n- New: `[workspace]` with `members = [\\\"a\\\", \\\"b\\\"]` and optional `[workspace.dependencies]`\\n\\n# Errors\\nReturns an `io::Error` if the file cannot be read or the TOML is invalid.\"},\"filterText\":\"read_pubspec_toml\",\"kind\":\"function\",\"label\":\"read_pubspec_toml\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"read_pubspec_toml\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(impl AsRef<Path>, &Pubspec) -> Result<(), Box<dyn Error + 'static, Global>>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Serializes a `PubspecObject` and writes it to `pubspec.toml` in the given directory.\\n\\nThis builds an ordered, None-filtered representation (reusing the YAML builder + null remover),\\nthen reshapes `workspace` into a TOML table with `members` and optional `dependencies`\\nwhen applicable, and finally serializes it to pretty TOML.\\n\\n# Errors\\nReturns an `io::Error` if serialization fails or the file cannot be written.\"},\"filterText\":\"write_pubspec_toml\",\"kind\":\"function\",\"label\":\"write_pubspec_toml\",\"sortText\":\"7ffffff6\",\"textEdit\":{\"newText\":\"write_pubspec_toml\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(self) -> T\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Converts this type into the (usually inferred) input type.\"},\"filterText\":\"into\",\"kind\":\"method\",\"label\":\"into(as Into)\",\"sortText\":\"7fffffec\",\"textEdit\":{\"newText\":\"into\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(T) -> Result<Self, <Self as TryFrom<T>>::Error>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Performs the conversion.\"},\"filterText\":\"try_from\",\"kind\":\"function\",\"label\":\"try_from(as TryFrom)\",\"sortText\":\"7ffffffb\",\"textEdit\":{\"newText\":\"try_from\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(self) -> Result<T, <Self as TryInto<T>>::Error>\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Performs the conversion.\"},\"filterText\":\"try_into\",\"kind\":\"method\",\"label\":\"try_into(as TryInto)\",\"sortText\":\"7ffffffb\",\"textEdit\":{\"newText\":\"try_into\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}},{\"detail\":\"fn(T) -> Self\",\"documentation\":{\"kind\":\"markdown\",\"value\":\"Converts to this type from the input type.\"},\"filterText\":\"from\",\"kind\":\"function\",\"label\":\"from(as From)\",\"sortText\":\"7fffffec\",\"textEdit\":{\"newText\":\"from\",\"range\":{\"end\":{\"column\":48,\"line\":26},\"start\":{\"column\":48,\"line\":26}}}}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 8. `aimer_rust_document_highlights`

**Purpose:** Highlights for the function symbol.  
**Elapsed:** 0.506s  
**Observed:** results=1

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "line": 25,
  "column": 18
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_document_highlights",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "line": 25,
      "column": 18
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-10"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 10,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"range\":{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}}}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 9. `aimer_rust_expand_macro`

**Purpose:** Expand an ordinary vec! invocation.  
**Elapsed:** 3.312s  
**Observed:** returned structured JSON

**Arguments**

````json
{
  "file": "bamboo/src/test.rs",
  "line": 35,
  "column": 48
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_expand_macro",
    "arguments": {
      "file": "bamboo/src/test.rs",
      "line": 35,
      "column": 48
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-11"
    }
  }
}
````

**Raw progress notifications**

````json
[
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer is loading and indexing the workspace; wait for this tool to finish.",
      "progress": 1.0,
      "progressToken": "aimer-lsp-report-11"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer reports no pending workspace work; confirming stable readiness (health=warning): Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.cargo check failed to start: Cargo watcher failed, the command produced no valid metadata (exit code: ExitStatus(unix_wait_status(25856))):\nwarning: profiles for the non root package will be ignored, specify profiles at the workspace root:\npackage:   /Users/cottons/Documents/bamboo/bamboo/Cargo.toml\nworkspace: /Users/cottons/Documents/bamboo/Cargo.toml\nwarning: virtual workspace defaulting to `resolver = \"1\"` despite one or more workspace members being on edition 2024 which implies `resolver = \"3\"`\n  |\n  = note: to keep the current resolver, specify `workspace.resolver = \"1\"` in the workspace root's manifest\n  = note: to use the edition 2024 resolver, specify `workspace.resolver = \"3\"` in the workspace root's manifest\n  = note: for more details see https://doc.rust-lang.org/cargo/reference/resolver.html#resolver-versions\nwarning: Cargo.toml: unused manifest key: workspace.workspace\nwarning: workspace (manifest) generated 1 warning\nerror: failed to open: /Users/cottons/Documents/bamboo/target/debug/.cargo-build-lock\n\nCaused by:\n  Operation not permitted (os error 1)",
      "progress": 2.0,
      "progressToken": "aimer-lsp-report-11"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer workspace is ready with a warning: Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.cargo check failed to start: Cargo watcher failed, the command produced no valid metadata (exit code: ExitStatus(unix_wait_status(25856))):\nwarning: profiles for the non root package will be ignored, specify profiles at the workspace root:\npackage:   /Users/cottons/Documents/bamboo/bamboo/Cargo.toml\nworkspace: /Users/cottons/Documents/bamboo/Cargo.toml\nwarning: virtual workspace defaulting to `resolver = \"1\"` despite one or more workspace members being on edition 2024 which implies `resolver = \"3\"`\n  |\n  = note: to keep the current resolver, specify `workspace.resolver = \"1\"` in the workspace root's manifest\n  = note: to use the edition 2024 resolver, specify `workspace.resolver = \"3\"` in the workspace root's manifest\n  = note: for more details see https://doc.rust-lang.org/cargo/reference/resolver.html#resolver-versions\nwarning: Cargo.toml: unused manifest key: workspace.workspace\nwarning: workspace (manifest) generated 1 warning\nerror: failed to open: /Users/cottons/Documents/bamboo/target/debug/.cargo-build-lock\n\nCaused by:\n  Operation not permitted (os error 1)",
      "progress": 3.0,
      "progressToken": "aimer-lsp-report-11"
    }
  }
]
````

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 11,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"expansion\":\"alloc::boxed::box_assume_init_into_vec_unsafe(alloc::intrinsics::write_box_via_move(\\n    alloc::boxed::Box::new_uninit(),\\n    [\\\"search\\\"],\\n))\\n\",\"name\":\"vec!\"}"
      }
    ],
    "isError": false
  }
}
````

### 10. `aimer_rust_find_declaration`

**Purpose:** Declaration from an actual call site.  
**Elapsed:** 3.304s  
**Observed:** results=1

**Arguments**

````json
{
  "file": "bamboo_core/src/bamboo_util.rs",
  "line": 24,
  "column": 33
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 12,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_find_declaration",
    "arguments": {
      "file": "bamboo_core/src/bamboo_util.rs",
      "line": 24,
      "column": 33
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-12"
    }
  }
}
````

**Raw progress notifications**

````json
[
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer is loading and indexing the workspace; wait for this tool to finish.",
      "progress": 1.0,
      "progressToken": "aimer-lsp-report-12"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer reports no pending workspace work; confirming stable readiness (health=warning): Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.cargo check failed to start: Cargo watcher failed, the command produced no valid metadata (exit code: ExitStatus(unix_wait_status(25856))):\nwarning: profiles for the non root package will be ignored, specify profiles at the workspace root:\npackage:   /Users/cottons/Documents/bamboo/bamboo/Cargo.toml\nworkspace: /Users/cottons/Documents/bamboo/Cargo.toml\nwarning: virtual workspace defaulting to `resolver = \"1\"` despite one or more workspace members being on edition 2024 which implies `resolver = \"3\"`\n  |\n  = note: to keep the current resolver, specify `workspace.resolver = \"1\"` in the workspace root's manifest\n  = note: to use the edition 2024 resolver, specify `workspace.resolver = \"3\"` in the workspace root's manifest\n  = note: for more details see https://doc.rust-lang.org/cargo/reference/resolver.html#resolver-versions\nwarning: Cargo.toml: unused manifest key: workspace.workspace\nwarning: workspace (manifest) generated 1 warning\nerror: failed to open: /Users/cottons/Documents/bamboo/target/debug/.cargo-build-lock\n\nCaused by:\n  Operation not permitted (os error 1)",
      "progress": 2.0,
      "progressToken": "aimer-lsp-report-12"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer workspace is ready with a warning: Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.cargo check failed to start: Cargo watcher failed, the command produced no valid metadata (exit code: ExitStatus(unix_wait_status(25856))):\nwarning: profiles for the non root package will be ignored, specify profiles at the workspace root:\npackage:   /Users/cottons/Documents/bamboo/bamboo/Cargo.toml\nworkspace: /Users/cottons/Documents/bamboo/Cargo.toml\nwarning: virtual workspace defaulting to `resolver = \"1\"` despite one or more workspace members being on edition 2024 which implies `resolver = \"3\"`\n  |\n  = note: to keep the current resolver, specify `workspace.resolver = \"1\"` in the workspace root's manifest\n  = note: to use the edition 2024 resolver, specify `workspace.resolver = \"3\"` in the workspace root's manifest\n  = note: for more details see https://doc.rust-lang.org/cargo/reference/resolver.html#resolver-versions\nwarning: Cargo.toml: unused manifest key: workspace.workspace\nwarning: workspace (manifest) generated 1 warning\nerror: failed to open: /Users/cottons/Documents/bamboo/target/debug/.cargo-build-lock\n\nCaused by:\n  Operation not permitted (os error 1)",
      "progress": 3.0,
      "progressToken": "aimer-lsp-report-12"
    }
  }
]
````

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 12,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"originSelectionRange\":{\"end\":{\"character\":43,\"line\":23},\"start\":{\"character\":26,\"line\":23}},\"target_file\":\"bamboo_core/src/file_reader.rs\",\"target_range\":{\"end\":{\"column\":6,\"line\":28},\"start\":{\"column\":5,\"line\":21}},\"target_selection_range\":{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}}}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 11. `aimer_rust_find_definition`

**Purpose:** Definition from an actual call site.  
**Elapsed:** 0.507s  
**Observed:** results=1

**Arguments**

````json
{
  "file": "bamboo_core/src/bamboo_util.rs",
  "line": 24,
  "column": 33
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 13,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_find_definition",
    "arguments": {
      "file": "bamboo_core/src/bamboo_util.rs",
      "line": 24,
      "column": 33
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-13"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 13,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"originSelectionRange\":{\"end\":{\"character\":43,\"line\":23},\"start\":{\"character\":26,\"line\":23}},\"target_file\":\"bamboo_core/src/file_reader.rs\",\"target_range\":{\"end\":{\"column\":6,\"line\":28},\"start\":{\"column\":5,\"line\":21}},\"target_selection_range\":{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}}}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 12. `aimer_rust_find_implementations`

**Purpose:** Implementations for the Display trait reference.  
**Elapsed:** 3.579s  
**Observed:** results=200 (truncated)

**Arguments**

````json
{
  "file": "bamboo_core/src/pubspec.rs",
  "line": 22,
  "column": 9
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 14,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_find_implementations",
    "arguments": {
      "file": "bamboo_core/src/pubspec.rs",
      "line": 22,
      "column": 9
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-14"
    }
  }
}
````

**Raw progress notifications**

````json
[
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer is loading and indexing the workspace; wait for this tool to finish.",
      "progress": 1.0,
      "progressToken": "aimer-lsp-report-14"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer reports no pending workspace work; confirming stable readiness (health=warning): Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.cargo check failed to start: Cargo watcher failed, the command produced no valid metadata (exit code: ExitStatus(unix_wait_status(25856))):\nwarning: profiles for the non root package will be ignored, specify profiles at the workspace root:\npackage:   /Users/cottons/Documents/bamboo/bamboo/Cargo.toml\nworkspace: /Users/cottons/Documents/bamboo/Cargo.toml\nwarning: virtual workspace defaulting to `resolver = \"1\"` despite one or more workspace members being on edition 2024 which implies `resolver = \"3\"`\n  |\n  = note: to keep the current resolver, specify `workspace.resolver = \"1\"` in the workspace root's manifest\n  = note: to use the edition 2024 resolver, specify `workspace.resolver = \"3\"` in the workspace root's manifest\n  = note: for more details see https://doc.rust-lang.org/cargo/reference/resolver.html#resolver-versions\nwarning: Cargo.toml: unused manifest key: workspace.workspace\nwarning: workspace (manifest) generated 1 warning\nerror: failed to open: /Users/cottons/Documents/bamboo/target/debug/.cargo-build-lock\n\nCaused by:\n  Operation not permitted (os error 1)",
      "progress": 2.0,
      "progressToken": "aimer-lsp-report-14"
    }
  },
  {
    "jsonrpc": "2.0",
    "method": "notifications/progress",
    "params": {
      "message": "rust-analyzer workspace is ready with a warning: Failed to run build scripts of some packages.\n\nPlease refer to the language server logs for more details on the errors.cargo check failed to start: Cargo watcher failed, the command produced no valid metadata (exit code: ExitStatus(unix_wait_status(25856))):\nwarning: profiles for the non root package will be ignored, specify profiles at the workspace root:\npackage:   /Users/cottons/Documents/bamboo/bamboo/Cargo.toml\nworkspace: /Users/cottons/Documents/bamboo/Cargo.toml\nwarning: virtual workspace defaulting to `resolver = \"1\"` despite one or more workspace members being on edition 2024 which implies `resolver = \"3\"`\n  |\n  = note: to keep the current resolver, specify `workspace.resolver = \"1\"` in the workspace root's manifest\n  = note: to use the edition 2024 resolver, specify `workspace.resolver = \"3\"` in the workspace root's manifest\n  = note: for more details see https://doc.rust-lang.org/cargo/reference/resolver.html#resolver-versions\nwarning: Cargo.toml: unused manifest key: workspace.workspace\nwarning: workspace (manifest) generated 1 warning\nerror: failed to open: /Users/cottons/Documents/bamboo/target/debug/.cargo-build-lock\n\nCaused by:\n  Operation not permitted (os error 1)",
      "progress": 3.0,
      "progressToken": "aimer-lsp-report-14"
    }
  }
]
````

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 14,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bitflags-2.13.2/src/lib.rs\",\"target_range\":{\"end\":{\"column\":33,\"line\":544},\"start\":{\"column\":17,\"line\":544}},\"target_selection_range\":{\"end\":{\"column\":33,\"line\":544},\"start\":{\"column\":17,\"line\":544}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustix-1.1.5/src/termios/types.rs\",\"target_range\":{\"end\":{\"column\":10,\"line\":245},\"start\":{\"column\":1,\"line\":245}},\"target_selection_range\":{\"end\":{\"column\":10,\"line\":245},\"start\":{\"column\":1,\"line\":245}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustix-1.1.5/src/ugid.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":131},\"start\":{\"column\":1,\"line\":127}},\"target_selection_range\":{\"end\":{\"column\":26,\"line\":127},\"start\":{\"column\":23,\"line\":127}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustix-1.1.5/src/termios/types.rs\",\"target_range\":{\"end\":{\"column\":10,\"line\":528},\"start\":{\"column\":1,\"line\":528}},\"target_selection_range\":{\"end\":{\"column\":10,\"line\":528},\"start\":{\"column\":1,\"line\":528}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustix-1.1.5/src/termios/types.rs\",\"target_range\":{\"end\":{\"column\":10,\"line\":594},\"start\":{\"column\":1,\"line\":594}},\"target_selection_range\":{\"end\":{\"column\":10,\"line\":594},\"start\":{\"column\":1,\"line\":594}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustix-1.1.5/src/termios/types.rs\",\"target_range\":{\"end\":{\"column\":10,\"line\":313},\"start\":{\"column\":1,\"line\":313}},\"target_selection_range\":{\"end\":{\"column\":10,\"line\":313},\"start\":{\"column\":1,\"line\":313}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustix-1.1.5/src/ugid.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":95},\"start\":{\"column\":1,\"line\":91}},\"target_selection_range\":{\"end\":{\"column\":26,\"line\":91},\"start\":{\"column\":23,\"line\":91}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustix-1.1.5/src/pid.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":109},\"start\":{\"column\":1,\"line\":105}},\"target_selection_range\":{\"end\":{\"column\":26,\"line\":105},\"start\":{\"column\":23,\"line\":105}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/either-1.18.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":53,\"line\":1640},\"start\":{\"column\":41,\"line\":1640}},\"target_selection_range\":{\"end\":{\"column\":53,\"line\":1640},\"start\":{\"column\":41,\"line\":1640}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/either-1.18.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":17,\"line\":1653},\"start\":{\"column\":1,\"line\":1653}},\"target_selection_range\":{\"end\":{\"column\":17,\"line\":1653},\"start\":{\"column\":1,\"line\":1653}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/utc_date_time.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1313},\"start\":{\"column\":1,\"line\":1304}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":1304},\"start\":{\"column\":23,\"line\":1304}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/error/conversion_range.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":17},\"start\":{\"column\":1,\"line\":12}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":12},\"start\":{\"column\":23,\"line\":12}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/month.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":250},\"start\":{\"column\":1,\"line\":245}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":245},\"start\":{\"column\":23,\"line\":245}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/offset_date_time.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1663},\"start\":{\"column\":1,\"line\":1654}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":1654},\"start\":{\"column\":23,\"line\":1654}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/error/invalid_variant.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":15},\"start\":{\"column\":1,\"line\":10}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":10},\"start\":{\"column\":23,\"line\":10}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/time.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1077},\"start\":{\"column\":1,\"line\":1068}},\"target_selection_range\":{\"end\":{\"column\":27,\"line\":1068},\"start\":{\"column\":23,\"line\":1068}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/utc_offset.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":609},\"start\":{\"column\":1,\"line\":600}},\"target_selection_range\":{\"end\":{\"column\":32,\"line\":600},\"start\":{\"column\":23,\"line\":600}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/date.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1652},\"start\":{\"column\":1,\"line\":1643}},\"target_selection_range\":{\"end\":{\"column\":27,\"line\":1643},\"start\":{\"column\":23,\"line\":1643}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/error/different_variant.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":15},\"start\":{\"column\":1,\"line\":10}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":10},\"start\":{\"column\":23,\"line\":10}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/plain_date_time.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1178},\"start\":{\"column\":1,\"line\":1169}},\"target_selection_range\":{\"end\":{\"column\":36,\"line\":1169},\"start\":{\"column\":23,\"line\":1169}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/timestamp.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1441},\"start\":{\"column\":1,\"line\":1432}},\"target_selection_range\":{\"end\":{\"column\":32,\"line\":1432},\"start\":{\"column\":23,\"line\":1432}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/weekday.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":229},\"start\":{\"column\":1,\"line\":224}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":224},\"start\":{\"column\":23,\"line\":224}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/error/component_range.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":57},\"start\":{\"column\":1,\"line\":52}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":52},\"start\":{\"column\":23,\"line\":52}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/error/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":107},\"start\":{\"column\":1,\"line\":84}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":84},\"start\":{\"column\":23,\"line\":84}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/error/indeterminate_offset.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":16},\"start\":{\"column\":1,\"line\":11}},\"target_selection_range\":{\"end\":{\"column\":42,\"line\":11},\"start\":{\"column\":23,\"line\":11}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/time-0.3.55/src/signed_duration.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1421},\"start\":{\"column\":1,\"line\":1332}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":1346},\"start\":{\"column\":23,\"line\":1346}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/percent-encoding-2.3.2/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":191},\"start\":{\"column\":1,\"line\":184}},\"target_selection_range\":{\"end\":{\"column\":40,\"line\":184},\"start\":{\"column\":23,\"line\":184}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-tls-0.6.0/src/client.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":198},\"start\":{\"column\":1,\"line\":194}},\"target_selection_range\":{\"end\":{\"column\":47,\"line\":194},\"start\":{\"column\":23,\"line\":194}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bytemuck-1.25.2/src/checked.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":226},\"start\":{\"column\":1,\"line\":221}},\"target_selection_range\":{\"end\":{\"column\":45,\"line\":222},\"start\":{\"column\":29,\"line\":222}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bytemuck-1.25.2/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":266},\"start\":{\"column\":1,\"line\":261}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":262},\"start\":{\"column\":29,\"line\":262}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/errno-0.3.14/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":69},\"start\":{\"column\":1,\"line\":56}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":56},\"start\":{\"column\":23,\"line\":56}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ipnet-2.12.2/src/ipnet.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":584},\"start\":{\"column\":1,\"line\":577}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":577},\"start\":{\"column\":23,\"line\":577}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ipnet-2.12.2/src/ipnet.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1383},\"start\":{\"column\":1,\"line\":1379}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":1379},\"start\":{\"column\":23,\"line\":1379}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ipnet-2.12.2/src/ipnet.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":129},\"start\":{\"column\":1,\"line\":125}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":125},\"start\":{\"column\":23,\"line\":125}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ipnet-2.12.2/src/ipnet.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":997},\"start\":{\"column\":1,\"line\":993}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":993},\"start\":{\"column\":23,\"line\":993}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ipnet-2.12.2/src/parser.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":349},\"start\":{\"column\":1,\"line\":345}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":345},\"start\":{\"column\":23,\"line\":345}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/mac_address-1.2.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":222},\"start\":{\"column\":1,\"line\":207}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":207},\"start\":{\"column\":28,\"line\":207}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/mac_address-1.2.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":78},\"start\":{\"column\":1,\"line\":71}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":71},\"start\":{\"column\":28,\"line\":71}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/mac_address-1.2.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":58},\"start\":{\"column\":1,\"line\":52}},\"target_selection_range\":{\"end\":{\"column\":43,\"line\":52},\"start\":{\"column\":28,\"line\":52}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/vendor/hashbrown-0.17.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":187},\"start\":{\"column\":1,\"line\":175}},\"target_selection_range\":{\"end\":{\"column\":44,\"line\":176},\"start\":{\"column\":29,\"line\":176}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/vendor/hashbrown-0.17.1/src/map.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":3033},\"start\":{\"column\":1,\"line\":3023}},\"target_selection_range\":{\"end\":{\"column\":89,\"line\":3023},\"start\":{\"column\":60,\"line\":3023}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/error/context.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":114},\"start\":{\"column\":1,\"line\":94}},\"target_selection_range\":{\"end\":{\"column\":40,\"line\":94},\"start\":{\"column\":28,\"line\":94}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/output/fmt.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":83},\"start\":{\"column\":1,\"line\":78}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":79},\"start\":{\"column\":28,\"line\":79}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/parser/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":55},\"start\":{\"column\":1,\"line\":38}},\"target_selection_range\":{\"end\":{\"column\":40,\"line\":38},\"start\":{\"column\":28,\"line\":38}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/builder/arg.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":4795},\"start\":{\"column\":1,\"line\":4790}},\"target_selection_range\":{\"end\":{\"column\":21,\"line\":4790},\"start\":{\"column\":18,\"line\":4790}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/builder/str.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":119},\"start\":{\"column\":1,\"line\":114}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":114},\"start\":{\"column\":28,\"line\":114}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/builder/command.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":5256},\"start\":{\"column\":1,\"line\":5252}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":5252},\"start\":{\"column\":23,\"line\":5252}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/error/kind.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":365},\"start\":{\"column\":1,\"line\":361}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":361},\"start\":{\"column\":28,\"line\":361}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/util/id.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":100},\"start\":{\"column\":1,\"line\":95}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":95},\"start\":{\"column\":28,\"line\":95}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/error/context.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":71},\"start\":{\"column\":1,\"line\":67}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":67},\"start\":{\"column\":28,\"line\":67}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/util/color.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":76},\"start\":{\"column\":1,\"line\":69}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":69},\"start\":{\"column\":28,\"line\":69}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/builder/range.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":204},\"start\":{\"column\":1,\"line\":192}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":192},\"start\":{\"column\":28,\"line\":192}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/error/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":854},\"start\":{\"column\":1,\"line\":843}},\"target_selection_range\":{\"end\":{\"column\":45,\"line\":843},\"start\":{\"column\":37,\"line\":843}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/error/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":941},\"start\":{\"column\":1,\"line\":936}},\"target_selection_range\":{\"end\":{\"column\":27,\"line\":937},\"start\":{\"column\":18,\"line\":937}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/builder/styled_str.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":221},\"start\":{\"column\":1,\"line\":212}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":213},\"start\":{\"column\":28,\"line\":213}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.7/src/util/escape.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":29},\"start\":{\"column\":1,\"line\":21}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":21},\"start\":{\"column\":28,\"line\":21}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hex-0.4.3/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":33},\"start\":{\"column\":1,\"line\":23}},\"target_selection_range\":{\"end\":{\"column\":35,\"line\":23},\"start\":{\"column\":23,\"line\":23}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/compact_str-0.9.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":2588},\"start\":{\"column\":1,\"line\":2584}},\"target_selection_range\":{\"end\":{\"column\":35,\"line\":2584},\"start\":{\"column\":23,\"line\":2584}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/compact_str-0.9.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":2518},\"start\":{\"column\":1,\"line\":2513}},\"target_selection_range\":{\"end\":{\"column\":32,\"line\":2513},\"start\":{\"column\":23,\"line\":2513}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/compact_str-0.9.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":2492},\"start\":{\"column\":1,\"line\":2488}},\"target_selection_range\":{\"end\":{\"column\":33,\"line\":2488},\"start\":{\"column\":23,\"line\":2488}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/compact_str-0.9.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":2300},\"start\":{\"column\":1,\"line\":2296}},\"target_selection_range\":{\"end\":{\"column\":36,\"line\":2296},\"start\":{\"column\":23,\"line\":2296}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/compact_str-0.9.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":2611},\"start\":{\"column\":1,\"line\":2604}},\"target_selection_range\":{\"end\":{\"column\":43,\"line\":2604},\"start\":{\"column\":23,\"line\":2604}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/powerfmt-0.2.0/src/buf.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":117},\"start\":{\"column\":1,\"line\":113}},\"target_selection_range\":{\"end\":{\"column\":59,\"line\":113},\"start\":{\"column\":42,\"line\":113}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.11.1/src/upgrade.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":289},\"start\":{\"column\":1,\"line\":285}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":285},\"start\":{\"column\":23,\"line\":285}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.11.1/src/ext/h1_reason_phrase.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":137},\"start\":{\"column\":1,\"line\":133}},\"target_selection_range\":{\"end\":{\"column\":47,\"line\":133},\"start\":{\"column\":28,\"line\":133}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.11.1/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":616},\"start\":{\"column\":1,\"line\":612}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":612},\"start\":{\"column\":23,\"line\":612}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.11.1/src/proto/h1/decode.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":687},\"start\":{\"column\":1,\"line\":683}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":683},\"start\":{\"column\":23,\"line\":683}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.11.1/src/body/length.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":108},\"start\":{\"column\":1,\"line\":99}},\"target_selection_range\":{\"end\":{\"column\":36,\"line\":99},\"start\":{\"column\":23,\"line\":99}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.11.1/src/proto/h2/ping.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":509},\"start\":{\"column\":1,\"line\":505}},\"target_selection_range\":{\"end\":{\"column\":40,\"line\":505},\"start\":{\"column\":23,\"line\":505}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.11.1/src/proto/h1/encode.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":426},\"start\":{\"column\":1,\"line\":422}},\"target_selection_range\":{\"end\":{\"column\":29,\"line\":422},\"start\":{\"column\":23,\"line\":422}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.11.1/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":697},\"start\":{\"column\":1,\"line\":693}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":693},\"start\":{\"column\":23,\"line\":693}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/native-tls-0.2.18/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":131},\"start\":{\"column\":1,\"line\":127}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":127},\"start\":{\"column\":23,\"line\":127}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/native-tls-0.2.18/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":298},\"start\":{\"column\":1,\"line\":288}},\"target_selection_range\":{\"end\":{\"column\":43,\"line\":288},\"start\":{\"column\":26,\"line\":288}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/native-tls-0.2.18/src/imp/security_framework.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":61},\"start\":{\"column\":1,\"line\":57}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":57},\"start\":{\"column\":23,\"line\":57}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/time.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":474},\"start\":{\"column\":1,\"line\":446}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":446},\"start\":{\"column\":23,\"line\":446}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/socket/addr.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":771},\"start\":{\"column\":1,\"line\":767}},\"target_selection_range\":{\"end\":{\"column\":52,\"line\":767},\"start\":{\"column\":23,\"line\":767}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/net/if_.rs\",\"target_range\":{\"end\":{\"column\":15,\"line\":42},\"start\":{\"column\":1,\"line\":42}},\"target_selection_range\":{\"end\":{\"column\":15,\"line\":42},\"start\":{\"column\":1,\"line\":42}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/socket/addr.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":630},\"start\":{\"column\":1,\"line\":621}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":621},\"start\":{\"column\":23,\"line\":621}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/net/if_.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":268},\"start\":{\"column\":1,\"line\":264}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":264},\"start\":{\"column\":23,\"line\":264}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/socket/addr.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1053},\"start\":{\"column\":1,\"line\":1040}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":1041},\"start\":{\"column\":23,\"line\":1041}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/socket/mod.rs\",\"target_range\":{\"end\":{\"column\":15,\"line\":316},\"start\":{\"column\":1,\"line\":316}},\"target_selection_range\":{\"end\":{\"column\":15,\"line\":316},\"start\":{\"column\":1,\"line\":316}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/socket/addr.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":895},\"start\":{\"column\":1,\"line\":880}},\"target_selection_range\":{\"end\":{\"column\":33,\"line\":881},\"start\":{\"column\":23,\"line\":881}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/time.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":720},\"start\":{\"column\":1,\"line\":694}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":694},\"start\":{\"column\":23,\"line\":694}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/socket/addr.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1426},\"start\":{\"column\":1,\"line\":1398}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":1398},\"start\":{\"column\":23,\"line\":1398}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/nix-0.30.1/src/sys/socket/mod.rs\",\"target_range\":{\"end\":{\"column\":15,\"line\":291},\"start\":{\"column\":1,\"line\":291}},\"target_selection_range\":{\"end\":{\"column\":15,\"line\":291},\"start\":{\"column\":1,\"line\":291}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tempfile-3.27.0/src/file/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":114},\"start\":{\"column\":1,\"line\":110}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":110},\"start\":{\"column\":23,\"line\":110}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tempfile-3.27.0/src/file/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":547},\"start\":{\"column\":1,\"line\":543}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":543},\"start\":{\"column\":26,\"line\":543}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tempfile-3.27.0/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":14},\"start\":{\"column\":1,\"line\":10}},\"target_selection_range\":{\"end\":{\"column\":32,\"line\":10},\"start\":{\"column\":23,\"line\":10}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/utf8_iter-1.0.4/src/report.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":36},\"start\":{\"column\":1,\"line\":32}},\"target_selection_range\":{\"end\":{\"column\":43,\"line\":32},\"start\":{\"column\":29,\"line\":32}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/anstream-1.0.0/src/adapter/strip.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":66},\"start\":{\"column\":1,\"line\":53}},\"target_selection_range\":{\"end\":{\"column\":43,\"line\":53},\"start\":{\"column\":28,\"line\":53}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/smallvec-1.16.2/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":318},\"start\":{\"column\":1,\"line\":314}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":314},\"start\":{\"column\":23,\"line\":314}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/simdutf8-0.1.5/src/basic.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":23},\"start\":{\"column\":1,\"line\":19}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":19},\"start\":{\"column\":29,\"line\":19}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/simdutf8-0.1.5/src/compat.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":65},\"start\":{\"column\":1,\"line\":49}},\"target_selection_range\":{\"end\":{\"column\":27,\"line\":49},\"start\":{\"column\":18,\"line\":49}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/pest_meta-2.9.2/src/ast.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":390},\"start\":{\"column\":1,\"line\":302}},\"target_selection_range\":{\"end\":{\"column\":33,\"line\":302},\"start\":{\"column\":29,\"line\":302}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/pest_meta-2.9.2/src/optimizer/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":372},\"start\":{\"column\":1,\"line\":288}},\"target_selection_range\":{\"end\":{\"column\":42,\"line\":288},\"start\":{\"column\":29,\"line\":288}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/getrandom-0.4.3/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":245},\"start\":{\"column\":1,\"line\":229}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":229},\"start\":{\"column\":23,\"line\":229}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/getrandom-0.3.4/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":212},\"start\":{\"column\":1,\"line\":196}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":196},\"start\":{\"column\":23,\"line\":196}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/mutex.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1036},\"start\":{\"column\":1,\"line\":1030}},\"target_selection_range\":{\"end\":{\"column\":35,\"line\":1031},\"start\":{\"column\":9,\"line\":1031}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/rwlock.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1849},\"start\":{\"column\":1,\"line\":1843}},\"target_selection_range\":{\"end\":{\"column\":35,\"line\":1844},\"start\":{\"column\":9,\"line\":1844}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/remutex.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":833},\"start\":{\"column\":1,\"line\":827}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":828},\"start\":{\"column\":9,\"line\":828}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/mutex.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":721},\"start\":{\"column\":1,\"line\":717}},\"target_selection_range\":{\"end\":{\"column\":96,\"line\":717},\"start\":{\"column\":76,\"line\":717}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/rwlock.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":3124},\"start\":{\"column\":1,\"line\":3118}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":3119},\"start\":{\"column\":9,\"line\":3119}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/rwlock.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1454},\"start\":{\"column\":1,\"line\":1448}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":1449},\"start\":{\"column\":9,\"line\":1449}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/rwlock.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":2946},\"start\":{\"column\":1,\"line\":2940}},\"target_selection_range\":{\"end\":{\"column\":40,\"line\":2941},\"start\":{\"column\":9,\"line\":2941}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/rwlock.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":2398},\"start\":{\"column\":1,\"line\":2392}},\"target_selection_range\":{\"end\":{\"column\":44,\"line\":2393},\"start\":{\"column\":9,\"line\":2393}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lock_api-0.4.14/src/remutex.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1133},\"start\":{\"column\":1,\"line\":1127}},\"target_selection_range\":{\"end\":{\"column\":47,\"line\":1128},\"start\":{\"column\":9,\"line\":1128}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustversion-1.0.23/src/date.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":22},\"start\":{\"column\":1,\"line\":14}},\"target_selection_range\":{\"end\":{\"column\":22,\"line\":14},\"start\":{\"column\":18,\"line\":14}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/deranged-0.5.8/src/lib.rs\",\"target_range\":{\"end\":{\"column\":13,\"line\":1762},\"start\":{\"column\":1,\"line\":1762}},\"target_selection_range\":{\"end\":{\"column\":13,\"line\":1762},\"start\":{\"column\":1,\"line\":1762}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/deranged-0.5.8/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":138},\"start\":{\"column\":1,\"line\":125}},\"target_selection_range\":{\"end\":{\"column\":36,\"line\":125},\"start\":{\"column\":23,\"line\":125}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/deranged-0.5.8/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":86},\"start\":{\"column\":1,\"line\":81}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":81},\"start\":{\"column\":23,\"line\":81}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/httparse-1.10.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":156},\"start\":{\"column\":1,\"line\":152}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":152},\"start\":{\"column\":23,\"line\":152}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/httparse-1.10.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":138},\"start\":{\"column\":1,\"line\":134}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":134},\"start\":{\"column\":23,\"line\":134}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/pest-2.9.2/src/iterators/pairs.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":481},\"start\":{\"column\":1,\"line\":466}},\"target_selection_range\":{\"end\":{\"column\":48,\"line\":466},\"start\":{\"column\":36,\"line\":466}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/pest-2.9.2/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":736},\"start\":{\"column\":1,\"line\":732}},\"target_selection_range\":{\"end\":{\"column\":44,\"line\":732},\"start\":{\"column\":36,\"line\":732}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/pest-2.9.2/src/parser_state.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":217},\"start\":{\"column\":1,\"line\":208}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":208},\"start\":{\"column\":18,\"line\":208}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/pest-2.9.2/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":745},\"start\":{\"column\":1,\"line\":738}},\"target_selection_range\":{\"end\":{\"column\":51,\"line\":738},\"start\":{\"column\":36,\"line\":738}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/pest-2.9.2/src/iterators/pair.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":368},\"start\":{\"column\":1,\"line\":341}},\"target_selection_range\":{\"end\":{\"column\":47,\"line\":341},\"start\":{\"column\":36,\"line\":341}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/zerovec-0.11.8/src/ule/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":427},\"start\":{\"column\":1,\"line\":416}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":416},\"start\":{\"column\":23,\"line\":416}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/zerovec-0.11.8/src/varzerovec/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":24},\"start\":{\"column\":1,\"line\":17}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":17},\"start\":{\"column\":18,\"line\":17}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/number.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":311},\"start\":{\"column\":1,\"line\":295}},\"target_selection_range\":{\"end\":{\"column\":24,\"line\":295},\"start\":{\"column\":18,\"line\":295}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/libyaml/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":84},\"start\":{\"column\":1,\"line\":65}},\"target_selection_range\":{\"end\":{\"column\":23,\"line\":65},\"start\":{\"column\":18,\"line\":65}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/path.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":34},\"start\":{\"column\":1,\"line\":13}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":13},\"start\":{\"column\":22,\"line\":13}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/libyaml/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":149},\"start\":{\"column\":1,\"line\":136}},\"target_selection_range\":{\"end\":{\"column\":22,\"line\":136},\"start\":{\"column\":18,\"line\":136}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/value/index.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":178},\"start\":{\"column\":1,\"line\":166}},\"target_selection_range\":{\"end\":{\"column\":35,\"line\":166},\"start\":{\"column\":27,\"line\":166}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/libyaml/cstr.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":54},\"start\":{\"column\":1,\"line\":47}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":47},\"start\":{\"column\":22,\"line\":47}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/value/tagged.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":166},\"start\":{\"column\":1,\"line\":162}},\"target_selection_range\":{\"end\":{\"column\":21,\"line\":162},\"start\":{\"column\":18,\"line\":162}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/mapping.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":851},\"start\":{\"column\":1,\"line\":838}},\"target_selection_range\":{\"end\":{\"column\":43,\"line\":838},\"start\":{\"column\":22,\"line\":838}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":160},\"start\":{\"column\":1,\"line\":156}},\"target_selection_range\":{\"end\":{\"column\":23,\"line\":156},\"start\":{\"column\":18,\"line\":156}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":192},\"start\":{\"column\":1,\"line\":183}},\"target_selection_range\":{\"end\":{\"column\":40,\"line\":183},\"start\":{\"column\":29,\"line\":183}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/line-clipping-0.3.8/src/cohen_sutherland.rs\",\"target_range\":{\"end\":{\"column\":10,\"line\":113},\"start\":{\"column\":1,\"line\":113}},\"target_selection_range\":{\"end\":{\"column\":10,\"line\":113},\"start\":{\"column\":1,\"line\":113}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/futures-channel-0.3.34/src/mpsc/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":202},\"start\":{\"column\":1,\"line\":194}},\"target_selection_range\":{\"end\":{\"column\":32,\"line\":194},\"start\":{\"column\":23,\"line\":194}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/futures-channel-0.3.34/src/mpsc/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":210},\"start\":{\"column\":1,\"line\":206}},\"target_selection_range\":{\"end\":{\"column\":32,\"line\":206},\"start\":{\"column\":23,\"line\":206}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/futures-channel-0.3.34/src/mpsc/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":258},\"start\":{\"column\":1,\"line\":250}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":250},\"start\":{\"column\":26,\"line\":250}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/futures-channel-0.3.34/src/oneshot.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":425},\"start\":{\"column\":1,\"line\":421}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":421},\"start\":{\"column\":23,\"line\":421}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/futures-channel-0.3.34/src/mpsc/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":291},\"start\":{\"column\":1,\"line\":284}},\"target_selection_range\":{\"end\":{\"column\":35,\"line\":284},\"start\":{\"column\":23,\"line\":284}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ucd-trie-0.1.7/src/owned.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":93},\"start\":{\"column\":1,\"line\":79}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":79},\"start\":{\"column\":23,\"line\":79}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/security-framework-3.7.0/src/cms.rs\",\"target_range\":{\"end\":{\"column\":24,\"line\":41},\"start\":{\"column\":5,\"line\":41}},\"target_selection_range\":{\"end\":{\"column\":24,\"line\":41},\"start\":{\"column\":5,\"line\":41}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/security-framework-3.7.0/src/base.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":83},\"start\":{\"column\":1,\"line\":74}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":74},\"start\":{\"column\":23,\"line\":74}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/security-framework-3.7.0/src/passwords_options.rs\",\"target_range\":{\"end\":{\"column\":20,\"line\":32},\"start\":{\"column\":1,\"line\":32}},\"target_selection_range\":{\"end\":{\"column\":20,\"line\":32},\"start\":{\"column\":1,\"line\":32}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/security-framework-3.7.0/src/policy.rs\",\"target_range\":{\"end\":{\"column\":20,\"line\":29},\"start\":{\"column\":1,\"line\":29}},\"target_selection_range\":{\"end\":{\"column\":20,\"line\":29},\"start\":{\"column\":1,\"line\":29}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/security-framework-3.7.0/src/authorization.rs\",\"target_range\":{\"end\":{\"column\":20,\"line\":43},\"start\":{\"column\":1,\"line\":43}},\"target_selection_range\":{\"end\":{\"column\":20,\"line\":43},\"start\":{\"column\":1,\"line\":43}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/security-framework-3.7.0/src/os/macos/code_signing.rs\",\"target_range\":{\"end\":{\"column\":20,\"line\":30},\"start\":{\"column\":1,\"line\":30}},\"target_selection_range\":{\"end\":{\"column\":20,\"line\":30},\"start\":{\"column\":1,\"line\":30}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/security-framework-3.7.0/src/trust.rs\",\"target_range\":{\"end\":{\"column\":20,\"line\":61},\"start\":{\"column\":1,\"line\":61}},\"target_selection_range\":{\"end\":{\"column\":20,\"line\":61},\"start\":{\"column\":1,\"line\":61}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/anstyle-1.0.14/src/reset.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":20},\"start\":{\"column\":1,\"line\":16}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":16},\"start\":{\"column\":29,\"line\":16}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/anstyle-1.0.14/src/color.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":632},\"start\":{\"column\":1,\"line\":627}},\"target_selection_range\":{\"end\":{\"column\":42,\"line\":627},\"start\":{\"column\":29,\"line\":627}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/anstyle-1.0.14/src/style.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":420},\"start\":{\"column\":1,\"line\":411}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":411},\"start\":{\"column\":29,\"line\":411}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/anstyle-1.0.14/src/effect.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":330},\"start\":{\"column\":1,\"line\":322}},\"target_selection_range\":{\"end\":{\"column\":43,\"line\":322},\"start\":{\"column\":29,\"line\":322}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/anstyle-1.0.14/src/color.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":642},\"start\":{\"column\":1,\"line\":637}},\"target_selection_range\":{\"end\":{\"column\":42,\"line\":637},\"start\":{\"column\":29,\"line\":637}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/anstyle-1.0.14/src/style.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":429},\"start\":{\"column\":1,\"line\":425}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":425},\"start\":{\"column\":29,\"line\":425}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.22.1/src/alphabet.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":163},\"start\":{\"column\":1,\"line\":154}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":154},\"start\":{\"column\":23,\"line\":154}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.22.1/src/display.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":38},\"start\":{\"column\":1,\"line\":33}},\"target_selection_range\":{\"end\":{\"column\":61,\"line\":33},\"start\":{\"column\":37,\"line\":33}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.22.1/src/decode.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":45},\"start\":{\"column\":1,\"line\":32}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":32},\"start\":{\"column\":23,\"line\":32}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.22.1/src/decode.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":66},\"start\":{\"column\":1,\"line\":59}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":59},\"start\":{\"column\":23,\"line\":59}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.22.1/src/encode.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":157},\"start\":{\"column\":1,\"line\":151}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":151},\"start\":{\"column\":23,\"line\":151}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.23.1/src/decode.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":77},\"start\":{\"column\":1,\"line\":46}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":46},\"start\":{\"column\":23,\"line\":46}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.23.1/src/alphabet.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":243},\"start\":{\"column\":1,\"line\":234}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":234},\"start\":{\"column\":23,\"line\":234}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.23.1/src/display.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":38},\"start\":{\"column\":1,\"line\":33}},\"target_selection_range\":{\"end\":{\"column\":61,\"line\":33},\"start\":{\"column\":37,\"line\":33}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.23.1/src/encode.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":161},\"start\":{\"column\":1,\"line\":155}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":155},\"start\":{\"column\":23,\"line\":155}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.23.1/src/decode.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":105},\"start\":{\"column\":1,\"line\":98}},\"target_selection_range\":{\"end\":{\"column\":39,\"line\":98},\"start\":{\"column\":23,\"line\":98}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ordered-float-4.6.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":359},\"start\":{\"column\":1,\"line\":354}},\"target_selection_range\":{\"end\":{\"column\":67,\"line\":354},\"start\":{\"column\":52,\"line\":354}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ordered-float-4.6.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1796},\"start\":{\"column\":1,\"line\":1792}},\"target_selection_range\":{\"end\":{\"column\":33,\"line\":1792},\"start\":{\"column\":23,\"line\":1792}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ordered-float-4.6.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1377},\"start\":{\"column\":1,\"line\":1372}},\"target_selection_range\":{\"end\":{\"column\":61,\"line\":1372},\"start\":{\"column\":52,\"line\":1372}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ordered-float-4.6.0/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1976},\"start\":{\"column\":1,\"line\":1969}},\"target_selection_range\":{\"end\":{\"column\":59,\"line\":1969},\"start\":{\"column\":40,\"line\":1969}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/core-foundation-0.10.1/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":40},\"start\":{\"column\":1,\"line\":36}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":36},\"start\":{\"column\":23,\"line\":36}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/core-foundation-0.10.1/src/string.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":106},\"start\":{\"column\":1,\"line\":102}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":102},\"start\":{\"column\":23,\"line\":102}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/yoke-0.8.3/src/yoke.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":118},\"start\":{\"column\":1,\"line\":110}},\"target_selection_range\":{\"end\":{\"column\":45,\"line\":110},\"start\":{\"column\":35,\"line\":110}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/syn-3.0.6/src/lifetime.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":93},\"start\":{\"column\":1,\"line\":88}},\"target_selection_range\":{\"end\":{\"column\":26,\"line\":88},\"start\":{\"column\":18,\"line\":88}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/syn-3.0.6/src/attr.rs\",\"target_range\":{\"end\":{\"column\":6,\"line\":776},\"start\":{\"column\":5,\"line\":769}},\"target_selection_range\":{\"end\":{\"column\":46,\"line\":769},\"start\":{\"column\":26,\"line\":769}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/syn-3.0.6/src/lit.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":499},\"start\":{\"column\":1,\"line\":495}},\"target_selection_range\":{\"end\":{\"column\":24,\"line\":495},\"start\":{\"column\":18,\"line\":495}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/syn-3.0.6/src/parse.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":281},\"start\":{\"column\":1,\"line\":277}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":277},\"start\":{\"column\":22,\"line\":277}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/syn-3.0.6/src/lookahead.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":166},\"start\":{\"column\":1,\"line\":154}},\"target_selection_range\":{\"end\":{\"column\":40,\"line\":154},\"start\":{\"column\":22,\"line\":154}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/syn-3.0.6/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":460},\"start\":{\"column\":1,\"line\":456}},\"target_selection_range\":{\"end\":{\"column\":23,\"line\":456},\"start\":{\"column\":18,\"line\":456}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/syn-3.0.6/src/lit.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":555},\"start\":{\"column\":1,\"line\":551}},\"target_selection_range\":{\"end\":{\"column\":26,\"line\":551},\"start\":{\"column\":18,\"line\":551}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/syn-3.0.6/src/attr.rs\",\"target_range\":{\"end\":{\"column\":6,\"line\":790},\"start\":{\"column\":5,\"line\":780}},\"target_selection_range\":{\"end\":{\"column\":41,\"line\":780},\"start\":{\"column\":26,\"line\":780}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_json-1.0.151/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":403},\"start\":{\"column\":1,\"line\":399}},\"target_selection_range\":{\"end\":{\"column\":23,\"line\":399},\"start\":{\"column\":18,\"line\":399}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_json-1.0.151/src/value/index.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":157},\"start\":{\"column\":1,\"line\":146}},\"target_selection_range\":{\"end\":{\"column\":30,\"line\":146},\"start\":{\"column\":22,\"line\":146}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_json-1.0.151/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":479},\"start\":{\"column\":1,\"line\":467}},\"target_selection_range\":{\"end\":{\"column\":40,\"line\":467},\"start\":{\"column\":22,\"line\":467}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_json-1.0.151/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":417},\"start\":{\"column\":1,\"line\":405}},\"target_selection_range\":{\"end\":{\"column\":27,\"line\":405},\"start\":{\"column\":18,\"line\":405}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_json-1.0.151/src/number.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":364},\"start\":{\"column\":1,\"line\":350}},\"target_selection_range\":{\"end\":{\"column\":24,\"line\":350},\"start\":{\"column\":18,\"line\":350}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_json-1.0.151/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":387},\"start\":{\"column\":1,\"line\":349}},\"target_selection_range\":{\"end\":{\"column\":27,\"line\":349},\"start\":{\"column\":18,\"line\":349}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_json-1.0.151/src/value/mod.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":257},\"start\":{\"column\":1,\"line\":197}},\"target_selection_range\":{\"end\":{\"column\":23,\"line\":197},\"start\":{\"column\":18,\"line\":197}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/log-0.4.34/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":541},\"start\":{\"column\":1,\"line\":537}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":537},\"start\":{\"column\":23,\"line\":537}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/log-0.4.34/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1590},\"start\":{\"column\":1,\"line\":1586}},\"target_selection_range\":{\"end\":{\"column\":37,\"line\":1586},\"start\":{\"column\":23,\"line\":1586}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/log-0.4.34/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":691},\"start\":{\"column\":1,\"line\":687}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":687},\"start\":{\"column\":23,\"line\":687}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/log-0.4.34/src/lib.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":1607},\"start\":{\"column\":1,\"line\":1603}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":1603},\"start\":{\"column\":23,\"line\":1603}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/regex-1.13.1/src/error.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":78},\"start\":{\"column\":1,\"line\":68}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":68},\"start\":{\"column\":29,\"line\":68}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/regex-1.13.1/src/regex/bytes.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":162},\"start\":{\"column\":1,\"line\":157}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":157},\"start\":{\"column\":29,\"line\":157}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/regex-1.13.1/src/regex/string.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":160},\"start\":{\"column\":1,\"line\":155}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":155},\"start\":{\"column\":29,\"line\":155}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/terminal/viewport.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":129},\"start\":{\"column\":1,\"line\":121}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":121},\"start\":{\"column\":23,\"line\":121}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/text/span.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":504},\"start\":{\"column\":1,\"line\":497}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":497},\"start\":{\"column\":23,\"line\":497}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/backend/test.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":245},\"start\":{\"column\":1,\"line\":239}},\"target_selection_range\":{\"end\":{\"column\":34,\"line\":239},\"start\":{\"column\":23,\"line\":239}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/layout/rect.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":149},\"start\":{\"column\":1,\"line\":145}},\"target_selection_range\":{\"end\":{\"column\":27,\"line\":145},\"start\":{\"column\":23,\"line\":145}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/text/masked.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":63},\"start\":{\"column\":1,\"line\":58}},\"target_selection_range\":{\"end\":{\"column\":33,\"line\":58},\"start\":{\"column\":23,\"line\":58}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/layout/position.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":118},\"start\":{\"column\":1,\"line\":114}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":114},\"start\":{\"column\":23,\"line\":114}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/style.rs\",\"target_range\":{\"end\":{\"column\":10,\"line\":87},\"start\":{\"column\":1,\"line\":87}},\"target_selection_range\":{\"end\":{\"column\":10,\"line\":87},\"start\":{\"column\":1,\"line\":87}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/text/line.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":846},\"start\":{\"column\":1,\"line\":839}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":839},\"start\":{\"column\":23,\"line\":839}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/style/color.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":255},\"start\":{\"column\":1,\"line\":251}},\"target_selection_range\":{\"end\":{\"column\":38,\"line\":251},\"start\":{\"column\":23,\"line\":251}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/layout/constraint.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":405},\"start\":{\"column\":1,\"line\":394}},\"target_selection_range\":{\"end\":{\"column\":33,\"line\":394},\"start\":{\"column\":23,\"line\":394}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/style/color.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":369},\"start\":{\"column\":1,\"line\":345}},\"target_selection_range\":{\"end\":{\"column\":28,\"line\":345},\"start\":{\"column\":23,\"line\":345}}},{\"originSelectionRange\":{\"end\":{\"character\":12,\"line\":21},\"start\":{\"character\":5,\"line\":21}},\"target_file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ratatui-core-0.1.2/src/text/text.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":760},\"start\":{\"column\":1,\"line\":750}},\"target_selection_range\":{\"end\":{\"column\":31,\"line\":750},\"start\":{\"column\":23,\"line\":750}}}],\"truncated\":true}"
      }
    ],
    "isError": false
  }
}
````

### 13. `aimer_rust_find_references`

**Purpose:** References to read_pubspec_yaml.  
**Elapsed:** 0.622s  
**Observed:** results=15

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "line": 25,
  "column": 18
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 15,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_find_references",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "line": 25,
      "column": 18
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-15"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 15,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"file\":\"bamboo_core/src/project_type.rs\",\"range\":{\"end\":{\"column\":51,\"line\":128},\"start\":{\"column\":34,\"line\":128}}},{\"file\":\"bamboo_core/src/project_type.rs\",\"range\":{\"end\":{\"column\":54,\"line\":161},\"start\":{\"column\":37,\"line\":161}}},{\"file\":\"bamboo_core/src/project_type.rs\",\"range\":{\"end\":{\"column\":71,\"line\":187},\"start\":{\"column\":54,\"line\":187}}},{\"file\":\"bamboo_core/src/package_cmd/cmd_status.rs\",\"range\":{\"end\":{\"column\":41,\"line\":34},\"start\":{\"column\":24,\"line\":34}}},{\"file\":\"bamboo_core/src/package_cmd/cmd_type.rs\",\"range\":{\"end\":{\"column\":45,\"line\":36},\"start\":{\"column\":28,\"line\":36}}},{\"file\":\"bamboo_core/src/package_cmd/cmd_run.rs\",\"range\":{\"end\":{\"column\":51,\"line\":60},\"start\":{\"column\":34,\"line\":60}}},{\"file\":\"bamboo_core/src/package_cmd/cmd_run.rs\",\"range\":{\"end\":{\"column\":72,\"line\":128},\"start\":{\"column\":55,\"line\":128}}},{\"file\":\"bamboo_core/src/package_cmd/cmd_init.rs\",\"range\":{\"end\":{\"column\":51,\"line\":70},\"start\":{\"column\":34,\"line\":70}}},{\"file\":\"bamboo_core/src/package_cmd/cmd_build.rs\",\"range\":{\"end\":{\"column\":46,\"line\":60},\"start\":{\"column\":29,\"line\":60}}},{\"file\":\"bamboo_core/src/package_cmd/cmd_build.rs\",\"range\":{\"end\":{\"column\":46,\"line\":75},\"start\":{\"column\":29,\"line\":75}}},{\"file\":\"bamboo_core/src/bamboo_util.rs\",\"range\":{\"end\":{\"column\":44,\"line\":24},\"start\":{\"column\":27,\"line\":24}}},{\"file\":\"bamboo_core/src/bamboo_util.rs\",\"range\":{\"end\":{\"column\":46,\"line\":264},\"start\":{\"column\":29,\"line\":264}}},{\"file\":\"bamboo_core/src/create_cmd/cmd_create.rs\",\"range\":{\"end\":{\"column\":58,\"line\":193},\"start\":{\"column\":41,\"line\":193}}},{\"file\":\"bamboo_core/src/create_cmd/cmd_create.rs\",\"range\":{\"end\":{\"column\":43,\"line\":226},\"start\":{\"column\":26,\"line\":226}}},{\"file\":\"bamboo_core/src/file_reader.rs\",\"range\":{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}}}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 14. `aimer_rust_format`

**Purpose:** Format preview; do not apply.  
**Elapsed:** 0.567s  
**Observed:** preview changed=true, files=1

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs"
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 16,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_format",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs"
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-16"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 16,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"changed\":true,\"files\":[{\"diff\":\"@@ line 202 @@\\n-            let value_map: Vec<Value> = workspace.members.iter().map(|s| Value::String(s.clone())).collect();\\n+            let value_map: Vec<Value> = workspace\\n+                .members\\n+                .iter()\\n+                .map(|s| Value::String(s.clone()))\\n+                .collect();\",\"file\":\"bamboo_core/src/file_reader.rs\",\"truncated\":false}],\"preview_id\":\"9036ff2e-3df0-454e-8e57-c2b9ba91ecb7\"}"
      }
    ],
    "isError": false
  }
}
````

### 15. `aimer_rust_get_call_hierarchy`

**Purpose:** Incoming call hierarchy.  
**Elapsed:** 0.765s  
**Observed:** results=13

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "line": 25,
  "column": 18,
  "direction": "incoming"
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 17,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_get_call_hierarchy",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "line": 25,
      "column": 18,
      "direction": "incoming"
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-17"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 17,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"from\":{\"detail\":\"async fn run(&self) -> Result<(), Box<dyn std::error::Error>>\",\"file\":\"bamboo_core/src/package_cmd/cmd_type.rs\",\"kind\":\"function\",\"name\":\"run\",\"range\":{\"end\":{\"column\":6,\"line\":79},\"start\":{\"column\":5,\"line\":30}},\"selection_range\":{\"end\":{\"column\":17,\"line\":30},\"start\":{\"column\":14,\"line\":30}}},\"fromRanges\":[{\"end\":{\"character\":44,\"line\":35},\"start\":{\"character\":27,\"line\":35}}]},{\"from\":{\"detail\":\"pub fn read_from_dir(dir: impl AsRef<Path>) -> Result<DartProjectType, Box<dyn std::error::Error>>\",\"file\":\"bamboo_core/src/project_type.rs\",\"kind\":\"function\",\"name\":\"read_from_dir\",\"range\":{\"end\":{\"column\":6,\"line\":148},\"start\":{\"column\":5,\"line\":75}},\"selection_range\":{\"end\":{\"column\":25,\"line\":80},\"start\":{\"column\":12,\"line\":80}}},\"fromRanges\":[{\"end\":{\"character\":50,\"line\":127},\"start\":{\"character\":33,\"line\":127}}]},{\"from\":{\"detail\":\"fn find_file_recursive(dir: &mut PathBuf) -> Result<Pubspec, Box<dyn std::error::Error>>\",\"file\":\"bamboo_core/src/project_type.rs\",\"kind\":\"function\",\"name\":\"find_file_recursive\",\"range\":{\"end\":{\"column\":6,\"line\":168},\"start\":{\"column\":5,\"line\":150}},\"selection_range\":{\"end\":{\"column\":27,\"line\":150},\"start\":{\"column\":8,\"line\":150}}},\"fromRanges\":[{\"end\":{\"character\":53,\"line\":160},\"start\":{\"character\":36,\"line\":160}}]},{\"from\":{\"detail\":\"pub fn is_workspace_child(pubspec_object: &Pubspec, current_dir: impl AsRef<Path>) -> Result<(Pubspec, &'static str, PathBuf), io::Error>\",\"file\":\"bamboo_core/src/project_type.rs\",\"kind\":\"function\",\"name\":\"is_workspace_child\",\"range\":{\"end\":{\"column\":6,\"line\":254},\"start\":{\"column\":5,\"line\":170}},\"selection_range\":{\"end\":{\"column\":30,\"line\":170},\"start\":{\"column\":12,\"line\":170}}},\"fromRanges\":[{\"end\":{\"character\":70,\"line\":186},\"start\":{\"character\":53,\"line\":186}}]},{\"from\":{\"detail\":\"async fn run(&self) -> Result<(), Box<dyn Error>>\",\"file\":\"bamboo_core/src/package_cmd/cmd_init.rs\",\"kind\":\"function\",\"name\":\"run\",\"range\":{\"end\":{\"column\":6,\"line\":122},\"start\":{\"column\":5,\"line\":38}},\"selection_range\":{\"end\":{\"column\":17,\"line\":38},\"start\":{\"column\":14,\"line\":38}}},\"fromRanges\":[{\"end\":{\"character\":50,\"line\":69},\"start\":{\"character\":33,\"line\":69}}]},{\"from\":{\"detail\":\"async fn run(&self) -> Result<(), Box<dyn Error>>\",\"file\":\"bamboo_core/src/package_cmd/cmd_build.rs\",\"kind\":\"function\",\"name\":\"run\",\"range\":{\"end\":{\"column\":3,\"line\":106},\"start\":{\"column\":2,\"line\":46}},\"selection_range\":{\"end\":{\"column\":14,\"line\":46},\"start\":{\"column\":11,\"line\":46}}},\"fromRanges\":[{\"end\":{\"character\":45,\"line\":59},\"start\":{\"character\":28,\"line\":59}},{\"end\":{\"character\":45,\"line\":74},\"start\":{\"character\":28,\"line\":74}}]},{\"from\":{\"detail\":\"fn create_flutter_project(&self) -> Result<(), Box<dyn Error>>\",\"file\":\"bamboo_core/src/create_cmd/cmd_create.rs\",\"kind\":\"function\",\"name\":\"create_flutter_project\",\"range\":{\"end\":{\"column\":3,\"line\":210},\"start\":{\"column\":2,\"line\":165}},\"selection_range\":{\"end\":{\"column\":27,\"line\":165},\"start\":{\"column\":5,\"line\":165}}},\"fromRanges\":[{\"end\":{\"character\":57,\"line\":192},\"start\":{\"character\":40,\"line\":192}}]},{\"from\":{\"detail\":\"fn create_workspace_child(&self, mode: &WorkspaceChildMode) -> Result<(), Box<dyn Error>>\",\"file\":\"bamboo_core/src/create_cmd/cmd_create.rs\",\"kind\":\"function\",\"name\":\"create_workspace_child\",\"range\":{\"end\":{\"column\":6,\"line\":266},\"start\":{\"column\":5,\"line\":212}},\"selection_range\":{\"end\":{\"column\":30,\"line\":212},\"start\":{\"column\":8,\"line\":212}}},\"fromRanges\":[{\"end\":{\"character\":42,\"line\":225},\"start\":{\"character\":25,\"line\":225}}]},{\"from\":{\"detail\":\"async fn run(&self) -> Result<(), Box<dyn Error>>\",\"file\":\"bamboo_core/src/package_cmd/cmd_status.rs\",\"kind\":\"function\",\"name\":\"run\",\"range\":{\"end\":{\"column\":3,\"line\":106},\"start\":{\"column\":2,\"line\":27}},\"selection_range\":{\"end\":{\"column\":14,\"line\":27},\"start\":{\"column\":11,\"line\":27}}},\"fromRanges\":[{\"end\":{\"character\":40,\"line\":33},\"start\":{\"character\":23,\"line\":33}}]},{\"from\":{\"detail\":\"fn new(args: &Vec<&str>, current_dir: &PathBuf) -> Result<Self, Box<dyn Error>>\\nwhere\\n    Self: Sized + Debug,\",\"file\":\"bamboo_core/src/package_cmd/cmd_run.rs\",\"kind\":\"function\",\"name\":\"new\",\"range\":{\"end\":{\"column\":6,\"line\":72},\"start\":{\"column\":5,\"line\":24}},\"selection_range\":{\"end\":{\"column\":11,\"line\":24},\"start\":{\"column\":8,\"line\":24}}},\"fromRanges\":[{\"end\":{\"character\":50,\"line\":59},\"start\":{\"character\":33,\"line\":59}}]},{\"from\":{\"detail\":\"fn get_current_project_type(current_dir: impl AsRef<Path>) -> Result<ProjectMode, Box<dyn Error>>\",\"file\":\"bamboo_core/src/package_cmd/cmd_run.rs\",\"kind\":\"function\",\"name\":\"get_current_project_type\",\"range\":{\"end\":{\"column\":6,\"line\":148},\"start\":{\"column\":5,\"line\":122}},\"selection_range\":{\"end\":{\"column\":32,\"line\":122},\"start\":{\"column\":8,\"line\":122}}},\"fromRanges\":[{\"end\":{\"character\":71,\"line\":127},\"start\":{\"character\":54,\"line\":127}}]},{\"from\":{\"detail\":\"pub fn validate_bamboo_file(&self, current_dir: &PathBuf) -> Result<bool, Box<dyn std::error::Error>>\\nwhere\\n    Self: Sized,\",\"file\":\"bamboo_core/src/bamboo_util.rs\",\"kind\":\"function\",\"name\":\"validate_bamboo_file\",\"range\":{\"end\":{\"column\":3,\"line\":66},\"start\":{\"column\":2,\"line\":15}},\"selection_range\":{\"end\":{\"column\":25,\"line\":15},\"start\":{\"column\":5,\"line\":15}}},\"fromRanges\":[{\"end\":{\"character\":43,\"line\":23},\"start\":{\"character\":26,\"line\":23}}]},{\"from\":{\"detail\":\"pub fn write_bamboo_file(&self, project_type: &DartProjectType, current_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>>\\nwhere\\n    Self: Sized,\",\"file\":\"bamboo_core/src/bamboo_util.rs\",\"kind\":\"function\",\"name\":\"write_bamboo_file\",\"range\":{\"end\":{\"column\":3,\"line\":293},\"start\":{\"column\":2,\"line\":234}},\"selection_range\":{\"end\":{\"column\":22,\"line\":234},\"start\":{\"column\":5,\"line\":234}}},\"fromRanges\":[{\"end\":{\"character\":45,\"line\":263},\"start\":{\"character\":28,\"line\":263}}]}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 16. `aimer_rust_get_code_actions`

**Purpose:** Code actions in the function range.  
**Elapsed:** 0.807s  
**Observed:** results=2

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "start_line": 21,
  "start_column": 1,
  "end_line": 28,
  "end_column": 1
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 18,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_get_code_actions",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "start_line": 21,
      "start_column": 1,
      "end_line": 28,
      "end_column": 1
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-18"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 18,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"action_id\":\"7de822b6-bb37-46f3-92b0-538ea78ec74e\",\"kind\":\"refactor.extract\",\"title\":\"Extract into variable\"},{\"action_id\":\"f1009d55-61cb-4d29-b6ee-bb0d84ba5ef1\",\"kind\":\"refactor.extract\",\"title\":\"Extract Module\"}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 17. `aimer_rust_get_code_lenses`

**Purpose:** Code lenses in the file.  
**Elapsed:** 0.505s  
**Observed:** results=0

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs"
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 19,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_get_code_lenses",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs"
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-19"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 19,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 18. `aimer_rust_hover`

**Purpose:** Hover on function name and documentation.  
**Elapsed:** 0.516s  
**Observed:** hover type and documentation returned

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "line": 25,
  "column": 18
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 20,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_hover",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "line": 25,
      "column": 18
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-20"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 20,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"contents\":{\"kind\":\"plaintext\",\"value\":\"bamboo_core::file_reader::PubspecFile\\n\\npub fn read_pubspec_yaml(dir: impl AsRef<Path>) -> io::Result<Pubspec>\\n\\n\\nReads the pubspec.yaml file from the given directory and deserializes it into PubspecObject.\\n\\nErrorsReturns an io::Error if the file cannot be read or if the YAML is invalid.\"},\"range\":{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}}}"
      }
    ],
    "isError": false
  }
}
````

### 19. `aimer_rust_inlay_hints`

**Purpose:** Inlay hints in the function range.  
**Elapsed:** 0.550s  
**Observed:** results=5

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "start_line": 21,
  "start_column": 1,
  "end_line": 28,
  "end_column": 1
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 21,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_inlay_hints",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "start_line": 21,
      "start_column": 1,
      "end_line": 28,
      "end_column": 1
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-21"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 21,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"kind\":\"type\",\"label\":[{\"value\":\": \"},{\"location\":{\"file\":\"/Users/cottons/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/alloc/src/string.rs\",\"range\":{\"end\":{\"column\":18,\"line\":353},\"start\":{\"column\":12,\"line\":353}}},\"value\":\"String\"}],\"paddingLeft\":false,\"paddingRight\":false,\"position\":{\"column\":20,\"line\":26},\"textEdits\":[{\"newText\":\": String\",\"range\":{\"end\":{\"column\":20,\"line\":26},\"start\":{\"column\":20,\"line\":26}}}]},{\"kind\":\"parameter\",\"label\":[{\"location\":{\"file\":\"/Users/cottons/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/std/src/fs.rs\",\"range\":{\"end\":{\"column\":43,\"line\":383},\"start\":{\"column\":39,\"line\":383}}},\"value\":\"path:\"}],\"paddingLeft\":false,\"paddingRight\":true,\"position\":{\"column\":42,\"line\":26}},{\"kind\":\"type\",\"label\":[{\"value\":\": \"},{\"location\":{\"file\":\"/Users/cottons/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_yaml-0.9.34+deprecated/src/error.rs\",\"range\":{\"end\":{\"column\":17,\"line\":12},\"start\":{\"column\":12,\"line\":12}}},\"value\":\"Error\"}],\"paddingLeft\":false,\"paddingRight\":false,\"position\":{\"column\":50,\"line\":27},\"textEdits\":[{\"newText\":\": serde_yaml::Error\",\"range\":{\"end\":{\"column\":50,\"line\":27},\"start\":{\"column\":50,\"line\":27}}}]},{\"kind\":\"parameter\",\"label\":[{\"location\":{\"file\":\"/Users/cottons/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/alloc/src/io/error.rs\",\"range\":{\"end\":{\"column\":23,\"line\":42},\"start\":{\"column\":19,\"line\":42}}},\"value\":\"kind:\"}],\"paddingLeft\":false,\"paddingRight\":true,\"position\":{\"column\":67,\"line\":27}},{\"kind\":\"parameter\",\"label\":[{\"location\":{\"file\":\"/Users/cottons/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/alloc/src/io/error.rs\",\"range\":{\"end\":{\"column\":41,\"line\":42},\"start\":{\"column\":36,\"line\":42}}},\"value\":\"error:\"}],\"paddingLeft\":false,\"paddingRight\":true,\"position\":{\"column\":95,\"line\":27}}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 20. `aimer_rust_prepare_rename`

**Purpose:** Prepare rename on function name.  
**Elapsed:** 0.507s  
**Observed:** rename target range returned

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "line": 25,
  "column": 18
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 22,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_prepare_rename",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "line": 25,
      "column": 18
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-22"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 22,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"end\":{\"column\":29,\"line\":25},\"start\":{\"column\":12,\"line\":25}}"
      }
    ],
    "isError": false
  }
}
````

### 21. `aimer_rust_rename_symbol`

**Purpose:** Rename preview; do not apply.  
**Elapsed:** 0.594s  
**Observed:** preview changed=true, files=9

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "line": 25,
  "column": 18,
  "new_name": "read_pubspec_yaml_lsp_test"
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 23,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_rename_symbol",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "line": 25,
      "column": 18,
      "new_name": "read_pubspec_yaml_lsp_test"
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-23"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 23,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"changed\":true,\"files\":[{\"diff\":\"@@ line 24 @@\\n-\\t\\t\\tErr(_) => PubspecFile::read_pubspec_yaml(current_dir)?,\\n-\\t\\t};\\n-\\n-\\t\\tlet bamboo_path = match DartProjectType::is_workspace_child(&pubspec_object, current_dir) {\\n-\\t\\t\\tOk(result) => {\\n-\\t\\t\\t\\t// println!(\\\"result : {:?}\\\", result);\\n-\\t\\t\\t\\tresult.2.join(\\\".bamboo\\\")\\n-\\t\\t\\t}\\n-\\n-\\t\\t\\tErr(_) => current_dir.join(\\\".bamboo\\\"),\\n-\\t\\t};\\n-\\n-\\t\\tif !bamboo_path.exists() {\\n-\\t\\t\\treturn Ok(false);\\n-\\t\\t}\\n-\\n-\\t\\tif let Ok(existing) = fs::read_to_string(&bamboo_path) {\\n-\\t\\t\\t// Extract signature and body (content without signature line)\\n-\\t\\t\\tlet mut extracted_sig: Option<String> = None;\\n-\\t\\t\\tlet mut body = String::new();\\n-\\t\\t\\tfor line in existing.lines() {\\n-\\t\\t\\t\\tlet trimmed = line.trim_start();\\n-\\t\\t\\t\\tif trimmed.starts_with(\\\"signature\\\") {\\n-\\t\\t\\t\\t\\tif let Some(start) = line.find('\\\"') {\\n-\\t\\t\\t\\t\\t\\tif let Some(end_rel) = line[start + 1..].find('\\\"') {\\n-\\t\\t\\t\\t\\t\\t\\tlet end = start + 1 + end_rel;\\n-\\t\\t\\t\\t\\t\\t\\textracted_sig = Some(line[start + 1..end].to_string());\\n-\\t\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t} else {\\n-\\t\\t\\t\\t\\tbody.push_str(line);\\n-\\t\\t\\t\\t\\tbody.push('\\\\n');\\n-\\t\\t\\t\\t}\\n-\\t\\t\\t}\\n-\\t\\t\\tif let Some(sig) = extracted_sig {\\n-\\t\\t\\t\\tif sig == Self::bamboo_signature_for(&body, BAMBOO_SIGNATURE_SALT) {\\n-\\t\\t\\t\\t\\treturn Ok(true);\\n-\\t\\t\\t\\t}\\n-\\t\\t\\t}\\n-\\t\\t}\\n-\\n-\\t\\tOk(false)\\n-\\t}\\n-\\n-\\tfn is_workspace_true(&self, v: &Value) -> Result<bool, Box<dyn std::error::Error>> {\\n-\\t\\tOk(true)\\n-\\t}\\n-\\n-\\tfn write_workspace_child_file(\\n-\\t\\t&self,\\n-\\t\\tparent: &Pubspec,\\n-\\t\\tchild: &mut Pubspec,\\n-\\t\\tchild_dir: &PathBuf,\\n-\\t\\tparent_dir: Option<&PathBuf>,\\n-\\t\\tcurrent_dir: &PathBuf,\\n-\\t) -> Result<(), Box<dyn std::error::Error>> {\\n-\\t\\tlet ws_deps = parent.workspace_dependencies.clone().unwrap_or_default();\\n-\\t\\tlet ws_dev_deps = parent\\n-\\t\\t\\t.workspace_dev_dependencies\\n-\\t\\t\\t.clone()\\n-\\t\\t\\t.unwrap_or_default();\\n-\\n-\\n-\\n-\\t\\tif let Some(ref mut deps) = child.dependencies {\\n-\\t\\t\\tfor (name, val) in deps.iter_mut() {\\n-\\t\\t\\t\\tif let Ok(true) = self.is_workspace_true(&serde_yaml::to_value(&val)?) {\\n-\\t\\t\\t\\t\\tif let Some(parent_ver) = ws_deps.get(name) {\\n-\\t\\t\\t\\t\\t\\t*val = parent_ver.clone();\\n-\\t\\t\\t\\t\\t\\tlogger!(\\n-\\t\\t\\t\\t\\t\\t\\t\\\"Replacing workspace marker for {} with version {:?}\\\",\\n-\\t\\t\\t\\t\\t\\t\\tname, parent_ver\\n-\\t\\t\\t\\t\\t\\t);\\n-\\t\\t\\t\\t\\t} else {\\n-\\t\\t\\t\\t\\t\\tlogger!(\\n-                            Error,\\n-                            \\\"workspace marker found for {} but no matching version in parent's workspace dependencies\\\",\\n-                            name\\n-                        );\\n-\\t\\t\\t\\t\\t\\tlogger!(Error, \\\"Error at path : {}\\\", child_dir.display());\\n-\\t\\t\\t\\t\\t\\treturn Err(\\\"[1] Error at parsing child yaml.\\\".into());\\n-\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t} else {\\n-\\t\\t\\t\\t\\tlogger!(\\n-                        Error,\\n-                        \\\"[2] Error at parsing child yaml. {name} : {:?}\\\",\\n-                        val\\n-                    );\\n-\\t\\t\\t\\t\\treturn Err(format!(\\\"[3] Error at parsing child yaml. {}\\\", name).into());\\n-\\t\\t\\t\\t}\\n-\\t\\t\\t}\\n-\\t\\t}\\n-\\n-\\t\\tfn path_depth_between(base: &PathBuf, target: &PathBuf) -> Option<usize> {\\n-\\t\\t\\tif let Ok(stripped) = target.strip_prefix(base) {\\n-\\t\\t\\t\\tSome(stripped.components().count())\\n-\\t\\t\\t} else {\\n-\\t\\t\\t\\tNone // target not under base\\n-\\t\\t\\t}\\n-\\t\\t}\\n-\\n-\\t\\t// Replace dev_dependencies workspace markers\\n-\\t\\tif let Some(ref mut dev_deps) = child.dev_dependencies {\\n-\\t\\t\\tfor (name, val) in dev_deps.iter_mut() {\\n-\\t\\t\\t\\tif let Ok(true) = self.is_workspace_true(&serde_yaml::to_value(&val)?) {\\n-\\t\\t\\t\\t\\tif let Some(parent_ver) = ws_dev_deps.get(name) {\\n-\\t\\t\\t\\t\\t\\t*val = parent_ver.clone();\\n-\\t\\t\\t\\t\\t\\tlogger!(\\n-\\t\\t\\t\\t\\t\\t\\t\\\"Replacing workspace marker for {} with version {:?}\\\",\\n-\\t\\t\\t\\t\\t\\t\\tname, parent_ver\\n-\\t\\t\\t\\t\\t\\t);\\n-\\t\\t\\t\\t\\t} else {\\n-\\t\\t\\t\\t\\t\\tlogger!(\\n-                            Error,\\n-                            \\\"workspace marker found for {} but no matching version in parent's workspace dev dependencies\\\",\\n-                            name\\n-                        );\\n-\\t\\t\\t\\t\\t\\tlogger!(Error, \\\"Error at path : {}\\\", child_dir.display());\\n-\\t\\t\\t\\t\\t\\treturn Err(\\\"[4] Error at parsing child yaml.\\\".into());\\n-\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t} else {\\n-\\t\\t\\t\\t\\tlogger!(\\n-                        Error,\\n-                        \\\"[5] Error at parsing child yaml. {name} : {:?}\\\",\\n-                        val\\n-                    );\\n-\\t\\t\\t\\t\\treturn Err(format!(\\\"[6] Error at parsing child yaml. {}\\\", name).into());\\n-\\t\\t\\t\\t}\\n-\\t\\t\\t}\\n-\\t\\t}\\n-\\n-\\t\\t// Adjust local path dependencies to be relative to the child folder depth.\\n-\\t\\t// For each inner folder level, prefix \\\"../\\\" when a dependency path is root-relative.\\n-\\t\\tlet depth = if let Some(parent_dir) = parent_dir\\n-\\t\\t\\t&& let Some(path_depth) = path_depth_between(parent_dir, child_dir)\\n-\\t\\t{\\n-\\t\\t\\tpath_depth\\n-\\t\\t} else if let Ok(rel) = child_dir.strip_prefix(current_dir) {\\n-\\t\\t\\t// Count only normal path components (exclude root/prefix markers)\\n-\\t\\t\\trel.components()\\n-\\t\\t\\t\\t.filter(|c| matches!(c, std::path::Component::Normal(_)))\\n-\\t\\t\\t\\t.count()\\n-\\t\\t} else {\\n-\\t\\t\\t0\\n-\\t\\t};\\n-\\n-\\t\\tif depth > 0 {\\n-\\t\\t\\tlogger!(\\\"Path depth for child is {}\\\", depth);\\n-\\t\\t\\tlet adjust_map = |map: &mut Dependencies| {\\n-\\t\\t\\t\\tfor (_name, val) in map.iter_mut() {\\n-\\t\\t\\t\\t\\tif let Some(source) = &val.source {\\n-\\t\\t\\t\\t\\t\\tmatch source {\\n-\\t\\t\\t\\t\\t\\t\\tDepSource::Path(path) => {\\n-\\t\\t\\t\\t\\t\\t\\t\\tlet mut p = path.clone();\\n-\\t\\t\\t\\t\\t\\t\\t\\tlet is_abs = Path::new(&path).is_absolute();\\n-\\t\\t\\t\\t\\t\\t\\t\\tlet starts_with_dot = p.starts_with(\\\"./\\\")\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\t|| p.starts_with(\\\"../\\\")\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\t|| p.starts_with(\\\".\\\\\\\\\\\")\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\t|| p.starts_with(\\\"..\\\\\\\\\\\");\\n-\\n-\\t\\t\\t\\t\\t\\t\\t\\tif !is_abs && starts_with_dot {\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\tif p.starts_with(\\\"./\\\") {\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\t\\tp = p.replace(\\\"./\\\", \\\"\\\");\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\tlet new_p = format!(\\\"{}{}\\\", \\\"../\\\".repeat(depth), p);\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\tlogger!(\\\"Adjusting path {} to {}\\\", p, new_p);\\n-\\t\\t\\t\\t\\t\\t\\t\\t\\tval.source = Some(DepSource::Path(new_p));\\n-\\t\\t\\t\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t\\t\\t\\t_ => ()\\n-\\t\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t\\t}\\n-\\t\\t\\t\\t}\\n-\\t\\t\\t};\\n-\\t\\t\\tif let Some(ref mut deps) = child.dependencies {\\n-\\t\\t\\t\\tadjust_map(deps);\\n-\\t\\t\\t}\\n-\\t\\t\\tif let Some(ref mut dev_deps) = child.dev_dependencies {\\n-\\t\\t\\t\\tadjust_map(dev_deps);\\n-\\t\\t\\t}\\n-\\t\\t}\\n-\\n-\\t\\t// If the child has a pubspec.yaml but no pubspec.toml, initialize the TOML from the parsed spec\\n-\\t\\tlet child_toml_path = PubspecFile::pubspec_toml_path(child_dir);\\n-\\t\\tlet child_yaml_path = PubspecFile::pubspec_path(child_dir);\\n-\\t\\tif !child_toml_path.exists() && child_yaml_path.exists() {\\n-\\t\\t\\tprintln!(\\n-\\t\\t\\t\\t\\\"Initializing pubspec.toml for workspace member at {}\\\",\\n-\\t\\t\\t\\tchild_dir.display()\\n-\\t\\t\\t);\\n-\\t\\t\\tPubspecFile::write_pubspec_toml(child_dir, &child)?;\\n-\\t\\t}\\n-\\n-\\t\\t// Write child's YAML\\n-\\t\\t// println!(\\\"Writing child YAML for {:?}\\\", child);\\n-\\t\\tPubspecFile::write_pubspec_yaml(child_dir, &child)?;\\n-\\n-\\t\\tOk(())\\n-\\t}\\n-\\n-\\tfn shorten_path(path: &str) -> String {\\n-\\t\\tif let Some(home) = env::home_dir() {\\n-\\t\\t\\tlet home_str = home.to_string_lossy();\\n-\\t\\t\\tif path.starts_with(&home_str.to_string()) {\\n-\\t\\t\\t\\treturn path.replacen(&home_str.to_string(), \\\"~\\\", 1);\\n-\\t\\t\\t}\\n-\\t\\t}\\n-\\t\\tpath.to_string()\\n-\\t}\\n-\\n-\\tfn write_bamboo_file(\\n-\\t\\t&self,\\n-\\t\\tproject_type: &DartProjectType,\\n-\\t\\tcurrent_dir: &PathBuf,\\n-\\t) -> Result<(), Box<dyn std::error::Error>>\\n-\\twhere\\n-\\t\\tSelf: Sized,\\n-\\t{\\n-\\t\\tlet ts = SystemTime::now()\\n-\\t\\t\\t.duration_since(UNIX_EPOCH)\\n-\\t\\t\\t.unwrap_or_default()\\n-\\t\\t\\t.as_secs();\\n-\\n-\\t\\tlet mut content = String::new();\\n-\\t\\tlet default_type = &ProjectMode::default();\\n-\\t\\tmatch project_type {\\n-\\t\\t\\tDartProjectType::Standard(_dir, pubspec) => {\\n-\\t\\t\\t\\tlet mode = pubspec.project_type.as_ref().unwrap_or(default_type);\\n-\\t\\t\\t\\tcontent.push_str(\\\"initialized = true\\\\n\\\");\\n-\\t\\t\\t\\tcontent.push_str(\\\"kind = \\\\\\\"standard\\\\\\\"\\\\n\\\");\\n-\\t\\t\\t\\tcontent.push_str(&format!(\\\"project_mode = \\\\\\\"{}\\\\\\\"\\\\n\\\", mode));\\n-\\t\\t\\t\\tcontent.push_str(&format!(\\\"unix_epoch = {}\\\\n\\\", ts));\\n-\\t\\t\\t\\tcontent.push_str(&format!(\\\"project_dir = \\\\\\\"{}\\\\\\\"\\\\n\\\", current_dir.display()));\\n-\\t\\t\\t}\\n-\\t\\t\\tDartProjectType::Workspace {\\n-\\t\\t\\t\\troot: _,\\n-\\t\\t\\t\\tmembers: children,\\n-\\t\\t\\t} => {\\n-\\t\\t\\t\\tlet root_mode = match PubspecFile::read_pubspec_toml(current_dir) {\\n-\\t\\t\\t\\t\\tOk(p) => p.project_type.unwrap_or(ProjectMode::default()),\\n-\\t\\t\\t\\t\\tErr(_) => PubspecFile::read_pubspec_yaml(current_dir)\\n+\\t\\t\\tErr(_) => PubspecFile::read_pubspec_yaml_lsp_test(current_dir)?,\\n+\\t\\t};\\n+\\n+\\t\\tlet bamboo_path = match DartProjectType::is_workspace_child(&pubspec_object, current_dir) {\\n+\\t\\t\\tOk(result) => {\\n+\\t\\t\\t\\t// println!(\\\"result : {:?}\\\", result);\\n+\\t\\t\\t\\tresult.2.join(\\\".bamboo\\\")\\n+\\t\\t\\t}\\n+\\n+\\t\\t\\tErr(_) => current_dir.join(\\\".bamboo\\\"),\\n+\\t\\t};\\n+\\n+\\t\\tif !bamboo_path.exists() {\\n+\\t\\t\\treturn Ok(false);\\n+\\t\\t}\\n+\\n+\\t\\tif let Ok(existing) = fs::read_to_string(&bamboo_path) {\\n+\\t\\t\\t// Extract signature and body (content without signature line)\\n+\\t\\t\\tlet mut extracted_sig: Option<String> = None;\\n+\\t\\t\\tlet mut body = String::new();\\n+\\t\\t\\tfor line in existing.lines() {\\n+\\t\\t\\t\\tlet trimmed = line.trim_start();\\n+\\t\\t\\t\\tif trimmed.starts_with(\\\"signature\\\") {\\n+\\t\\t\\t\\t\\tif let Some(start) = line.find('\\\"') {\\n+\\t\\t\\t\\t\\t\\tif let Some(end_rel) = line[start + 1..].find('\\\"') {\\n+\\t\\t\\t\\t\\t\\t\\tlet end = start + 1 + end_rel;\\n+\\t\\t\\t\\t\\t\\t\\textracted_sig = Some(line[start + 1..end].to_string());\\n+\\t\\t\\t\\t\\t\\t}\\n+\\t\\t\\t\\t\\t}\\n+\\t\\t\\t\\t} else {\\n+\\t\\t\\t\\t\\tbody.push_str(line);\\n+\\t\\t\\t\\t\\tbody.push('\\\\n');\\n+\\t\\t\\t\\t}\\n+\\t\\t\\t}\\n+\\t\\t\\tif let Some(sig) = extracted_sig {\\n+\\t\\t\\t\\tif sig == Self::bamboo_signature_for(&body, BAMBOO_SIGNATURE_SALT) {\\n+\\t\\t\\t\\t\\treturn Ok(true);\\n+\\t\\t\\t\\t}\\n+\\t\\t\\t}\\n+\\t\\t}\\n+\\n+\\t\\tOk(false)\\n+\\t}\\n+\\n+\\tfn is_workspace_true(&self, v: &Value) -> Result<bool, Box<dyn std::error::Error>> {\\n+\\t\\tOk(true)\\n+\\t}\\n+\\n+\\tfn write_workspace_child_file(\\n+\\t\\t&self,\\n+\\t\\tparent: &Pubspec,\\n+\\t\\tchild: &mut Pubspec,\\n+\\t\\tchild_dir: &PathBuf,\\n+\\t\\tparent_dir: Option<&PathBuf>,\\n+\\t\\tcurrent_dir: &PathBuf,\\n+\\t) -> Result<(), Box<dyn std::error::Error>> {\\n+\\t\\tlet ws_deps = parent.workspace_dependencies.clone().unwrap_or_default();\\n+\\t\\tlet ws_dev_deps = parent\",\"file\":\"bamboo_core/src/bamboo_util.rs\",\"truncated\":true},{\"diff\":\"@@ line 193 @@\\n-\\t\\tlet mut pubspec_object = PubspecFile::read_pubspec_yaml(&yaml_path)?;\\n-\\t\\tif self.create_mode.is_workspace_child() {\\n-\\t\\t\\tlet mut resolution_map = BTreeMap::new();\\n-\\t\\t\\tmatch resolution_map.insert(\\\"resolution\\\".to_string(), serde_yaml::Value::String(\\\"workspace\\\".to_string())){\\n-\\t\\t\\t\\tSome(_) => logger!(Debug, \\\"Resolution already exists\\\"),\\n-\\t\\t\\t\\tNone => logger!(Debug, \\\"Resolution added successfully\\\"),\\n-\\t\\t\\t};\\n-\\t\\t\\tpubspec_object.extra = resolution_map;\\n-\\t\\t}\\n-\\t\\tlet toml_path = self.current_dir.join(&self.name);\\n-\\n-\\t\\tPubspecFile::write_pubspec_toml(&toml_path, &pubspec_object)?;\\n-\\n-\\t\\tif self.parent_dir.is_none() {\\n-\\t\\t\\tself.write_bamboo_file(&DartProjectType::read_from_dir(self.current_dir.join(&self.name))?, &self.current_dir.join(&self.name))?;\\n-\\t\\t}\\n-\\t\\tOk(())\\n-\\t}\\n-\\n-    fn create_workspace_child(&self, mode: &WorkspaceChildMode) -> Result<(), Box<dyn Error>> {\\n-        type Child = WorkspaceChildMode;\\n-\\n-        logger!(\\n-            Debug,\\n-            \\\"DartProjectType::read_from_dir = {:?}\\\",\\n-            DartProjectType::read_from_dir(&self.current_dir)\\n-        );\\n-\\n-        let mut root_pubspec = if let Ok(pubspec_obj) =\\n-            PubspecFile::read_pubspec_toml(self.parent_dir.as_ref().unwrap())\\n-        {\\n-            pubspec_obj\\n-        } else if let Ok(pubspec_obj) =\\n-            PubspecFile::read_pubspec_yaml(self.parent_dir.as_ref().unwrap())\\n+\\t\\tlet mut pubspec_object = PubspecFile::read_pubspec_yaml_lsp_test(&yaml_path)?;\\n+\\t\\tif self.create_mode.is_workspace_child() {\\n+\\t\\t\\tlet mut resolution_map = BTreeMap::new();\\n+\\t\\t\\tmatch resolution_map.insert(\\\"resolution\\\".to_string(), serde_yaml::Value::String(\\\"workspace\\\".to_string())){\\n+\\t\\t\\t\\tSome(_) => logger!(Debug, \\\"Resolution already exists\\\"),\\n+\\t\\t\\t\\tNone => logger!(Debug, \\\"Resolution added successfully\\\"),\\n+\\t\\t\\t};\\n+\\t\\t\\tpubspec_object.extra = resolution_map;\\n+\\t\\t}\\n+\\t\\tlet toml_path = self.current_dir.join(&self.name);\\n+\\n+\\t\\tPubspecFile::write_pubspec_toml(&toml_path, &pubspec_object)?;\\n+\\n+\\t\\tif self.parent_dir.is_none() {\\n+\\t\\t\\tself.write_bamboo_file(&DartProjectType::read_from_dir(self.current_dir.join(&self.name))?, &self.current_dir.join(&self.name))?;\\n+\\t\\t}\\n+\\t\\tOk(())\\n+\\t}\\n+\\n+    fn create_workspace_child(&self, mode: &WorkspaceChildMode) -> Result<(), Box<dyn Error>> {\\n+        type Child = WorkspaceChildMode;\\n+\\n+        logger!(\\n+            Debug,\\n+            \\\"DartProjectType::read_from_dir = {:?}\\\",\\n+            DartProjectType::read_from_dir(&self.current_dir)\\n+        );\\n+\\n+        let mut root_pubspec = if let Ok(pubspec_obj) =\\n+            PubspecFile::read_pubspec_toml(self.parent_dir.as_ref().unwrap())\\n+        {\\n+            pubspec_obj\\n+        } else if let Ok(pubspec_obj) =\\n+            PubspecFile::read_pubspec_yaml_lsp_test(self.parent_dir.as_ref().unwrap())\",\"file\":\"bamboo_core/src/create_cmd/cmd_create.rs\",\"truncated\":false},{\"diff\":\"@@ line 25 @@\\n-    pub fn read_pubspec_yaml(dir: impl AsRef<Path>) -> io::Result<Pubspec> {\\n+    pub fn read_pubspec_yaml_lsp_test(dir: impl AsRef<Path>) -> io::Result<Pubspec> {\",\"file\":\"bamboo_core/src/file_reader.rs\",\"truncated\":false},{\"diff\":\"@@ line 60 @@\\n-\\t\\t\\t\\t\\tErr(_) => PubspecFile::read_pubspec_yaml(&self.current_dir)?,\\n-\\t\\t\\t\\t};\\n-\\t\\t\\t\\tlogger!(Info, \\\"Generating pubspec.yaml for {}\\\", root.name.as_ref().unwrap_or(&\\\"unknown\\\".to_string()));\\n-\\t\\t\\t\\tPubspecFile::write_pubspec_yaml(&self.current_dir, &root)?;\\n-\\t\\t\\t\\tlogger!(Info, \\\"Generating pubspec.yaml for {} - Completed\\\", root.name.as_ref().unwrap_or(&\\\"unknown\\\".to_string()));\\n-\\t\\t\\t}\\n-\\t\\t\\tDartProjectType::Workspace {\\n-\\t\\t\\t\\troot,\\n-\\t\\t\\t\\tmembers: children,\\n-\\t\\t\\t} => {\\n-\\t\\t\\t\\tlogger!(\\\"Workspace root : {}\\\", root.0.display());\\n-\\t\\t\\t\\tlogger!(Info, \\\"build workspace for workspace all members\\\");\\n-\\t\\t\\t\\t// Read root (TOML preferred) to get workspace dependencies\\n-\\t\\t\\t\\tlet root = match PubspecFile::read_pubspec_toml(&self.current_dir) {\\n-\\t\\t\\t\\t\\tOk(p) => p,\\n-\\t\\t\\t\\t\\tErr(_) => PubspecFile::read_pubspec_yaml(&self.current_dir)?,\\n+\\t\\t\\t\\t\\tErr(_) => PubspecFile::read_pubspec_yaml_lsp_test(&self.current_dir)?,\\n+\\t\\t\\t\\t};\\n+\\t\\t\\t\\tlogger!(Info, \\\"Generating pubspec.yaml for {}\\\", root.name.as_ref().unwrap_or(&\\\"unknown\\\".to_string()));\\n+\\t\\t\\t\\tPubspecFile::write_pubspec_yaml(&self.current_dir, &root)?;\\n+\\t\\t\\t\\tlogger!(Info, \\\"Generating pubspec.yaml for {} - Completed\\\", root.name.as_ref().unwrap_or(&\\\"unknown\\\".to_string()));\\n+\\t\\t\\t}\\n+\\t\\t\\tDartProjectType::Workspace {\\n+\\t\\t\\t\\troot,\\n+\\t\\t\\t\\tmembers: children,\\n+\\t\\t\\t} => {\\n+\\t\\t\\t\\tlogger!(\\\"Workspace root : {}\\\", root.0.display());\\n+\\t\\t\\t\\tlogger!(Info, \\\"build workspace for workspace all members\\\");\\n+\\t\\t\\t\\t// Read root (TOML preferred) to get workspace dependencies\\n+\\t\\t\\t\\tlet root = match PubspecFile::read_pubspec_toml(&self.current_dir) {\\n+\\t\\t\\t\\t\\tOk(p) => p,\\n+\\t\\t\\t\\t\\tErr(_) => PubspecFile::read_pubspec_yaml_lsp_test(&self.current_dir)?,\",\"file\":\"bamboo_core/src/package_cmd/cmd_build.rs\",\"truncated\":false},{\"diff\":\"@@ line 70 @@\\n-                    PubspecFile::read_pubspec_yaml(&self.current_dir)?\\n+                    PubspecFile::read_pubspec_yaml_lsp_test(&self.current_dir)?\",\"file\":\"bamboo_core/src/package_cmd/cmd_init.rs\",\"truncated\":false},{\"diff\":\"@@ line 60 @@\\n-                    PubspecFile::read_pubspec_yaml(&current_dir)?\\n-                }\\n-            }\\n-            DartProjectType::WorkSpaceChild { parent, .. } => parent,\\n-        };\\n-        Ok(Self {\\n-            binary_name,\\n-            args,\\n-            project_type,\\n-            current_dir: current_dir.to_path_buf(),\\n-            pubspec_object,\\n-        })\\n-    }\\n-\\n-    async fn run(&self) -> Result<(), Box<dyn Error>> {\\n-        BuildCmd::new(&vec![], &self.current_dir)\\n-            .map_err(|err| err.to_string())?\\n-            .run()\\n-            .await\\n-            .map_err(|err| err.to_string())?;\\n-\\n-        let script_name = match self.args.first() {\\n-            Some(script_name) => script_name,\\n-            None => return self.run_no_script(),\\n-        };\\n-\\n-        if let Some(scripts) = &self.pubspec_object.script {\\n-            let script_line = match scripts.get(script_name) {\\n-                Some(script) => script,\\n-                None => return self.run_no_script(),\\n-            };\\n-\\n-            let executable = {\\n-                #[cfg(target_os = \\\"windows\\\")]\\n-                {\\n-                    \\\"cmd\\\"\\n-                }\\n-                #[cfg(target_os = \\\"macos\\\")]\\n-                {\\n-                    \\\"sh\\\"\\n-                }\\n-                #[cfg(target_os = \\\"linux\\\")]\\n-                {\\n-                    \\\"sh\\\"\\n-                }\\n-            };\\n-            Command::new(executable)\\n-                .arg(\\\"-c\\\")\\n-                .arg(script_line)\\n-                .stdout(Stdio::inherit())\\n-                .stderr(Stdio::inherit())\\n-                .spawn()?\\n-                .wait()?;\\n-        } else {\\n-            return self.run_no_script();\\n-        };\\n-\\n-        Ok(())\\n-    }\\n-}\\n-\\n-impl RunCmd {\\n-    fn get_current_project_type(\\n-        current_dir: impl AsRef<Path>,\\n-    ) -> Result<ProjectMode, Box<dyn Error>> {\\n-        match PubspecFile::read_pubspec_toml(&current_dir) {\\n-            Ok(pubspec) => Ok(pubspec.project_type.unwrap_or(ProjectMode::default())),\\n-            Err(_) => {\\n-                let project_file = match PubspecFile::read_pubspec_yaml(&current_dir) {\\n+                    PubspecFile::read_pubspec_yaml_lsp_test(&current_dir)?\\n+                }\\n+            }\\n+            DartProjectType::WorkSpaceChild { parent, .. } => parent,\\n+        };\\n+        Ok(Self {\\n+            binary_name,\\n+            args,\\n+            project_type,\\n+            current_dir: current_dir.to_path_buf(),\\n+            pubspec_object,\\n+        })\\n+    }\\n+\\n+    async fn run(&self) -> Result<(), Box<dyn Error>> {\\n+        BuildCmd::new(&vec![], &self.current_dir)\\n+            .map_err(|err| err.to_string())?\\n+            .run()\\n+            .await\\n+            .map_err(|err| err.to_string())?;\\n+\\n+        let script_name = match self.args.first() {\\n+            Some(script_name) => script_name,\\n+            None => return self.run_no_script(),\\n+        };\\n+\\n+        if let Some(scripts) = &self.pubspec_object.script {\\n+            let script_line = match scripts.get(script_name) {\\n+                Some(script) => script,\\n+                None => return self.run_no_script(),\\n+            };\\n+\\n+            let executable = {\\n+                #[cfg(target_os = \\\"windows\\\")]\\n+                {\\n+                    \\\"cmd\\\"\\n+                }\\n+                #[cfg(target_os = \\\"macos\\\")]\\n+                {\\n+                    \\\"sh\\\"\\n+                }\\n+                #[cfg(target_os = \\\"linux\\\")]\\n+                {\\n+                    \\\"sh\\\"\\n+                }\\n+            };\\n+            Command::new(executable)\\n+                .arg(\\\"-c\\\")\\n+                .arg(script_line)\\n+                .stdout(Stdio::inherit())\\n+                .stderr(Stdio::inherit())\\n+                .spawn()?\\n+                .wait()?;\\n+        } else {\\n+            return self.run_no_script();\\n+        };\\n+\\n+        Ok(())\\n+    }\\n+}\\n+\\n+impl RunCmd {\\n+    fn get_current_project_type(\\n+        current_dir: impl AsRef<Path>,\\n+    ) -> Result<ProjectMode, Box<dyn Error>> {\\n+        match PubspecFile::read_pubspec_toml(&current_dir) {\\n+            Ok(pubspec) => Ok(pubspec.project_type.unwrap_or(ProjectMode::default())),\\n+            Err(_) => {\\n+                let project_file = match PubspecFile::read_pubspec_yaml_lsp_test(&current_dir) {\",\"file\":\"bamboo_core/src/package_cmd/cmd_run.rs\",\"truncated\":false},{\"diff\":\"@@ line 34 @@\\n-\\t\\t\\t\\tmatch PubspecFile::read_pubspec_yaml(&self.current_dir) {\\n+\\t\\t\\t\\tmatch PubspecFile::read_pubspec_yaml_lsp_test(&self.current_dir) {\",\"file\":\"bamboo_core/src/package_cmd/cmd_status.rs\",\"truncated\":false},{\"diff\":\"@@ line 36 @@\\n-\\t\\t\\t\\t    match PubspecFile::read_pubspec_yaml(&current_dir) {\\n+\\t\\t\\t\\t    match PubspecFile::read_pubspec_yaml_lsp_test(&current_dir) {\",\"file\":\"bamboo_core/src/package_cmd/cmd_type.rs\",\"truncated\":false},{\"diff\":\"@@ line 128 @@\\n-                    PubspecFile::read_pubspec_yaml(&child_dir)?\\n-                } else {\\n-                    return Err(format!(\\n-                        \\\"Workspace member missing pubspec in {}\\\",\\n-                        child_dir.display()\\n-                    )\\n-                    .into());\\n-                };\\n-\\n-                map.insert(child_dir, child_pubspec);\\n-            }\\n-\\n-            logger!(Debug, \\\"{fn_name} Returning Workspace type\\\");\\n-            return Ok(DartProjectType::Workspace {\\n-                root: (dir, root_pubspec),\\n-                members: map,\\n-            });\\n-        }\\n-        logger!(Debug, \\\"{fn_name} Cleaning up the method\\\");\\n-        Ok(DartProjectType::Standard(dir.to_path_buf(), root_pubspec))\\n-    }\\n-\\n-    fn find_file_recursive(dir: &mut PathBuf) -> Result<Pubspec, Box<dyn std::error::Error>> {\\n-        let root_dir = PathBuf::from(\\\"/\\\");\\n-\\n-        // Detect and load root pubspec (TOML preferred if both exist).\\n-        while *dir != root_dir {\\n-            logger!(Debug, \\\"dir = {}\\\", dir.display());\\n-            let yaml_path = PubspecFile::pubspec_path(&dir);\\n-            let toml_path = PubspecFile::pubspec_toml_path(&dir);\\n-            if toml_path.exists() {\\n-                return PubspecFile::read_pubspec_toml(&dir).map_err(|err| err.into());\\n-            } else if yaml_path.exists() {\\n-                return PubspecFile::read_pubspec_yaml(&dir).map_err(|err| err.into());\\n-            };\\n-\\n-            dir.pop();\\n-        }\\n-\\n-        Err(\\\"Not a dart project\\\".into())\\n-    }\\n-\\n-    pub fn is_workspace_child(\\n-        pubspec_object: &Pubspec,\\n-        current_dir: impl AsRef<Path>,\\n-    ) -> Result<(Pubspec, &'static str, PathBuf), io::Error> {\\n-        if pubspec_object.extra.contains_key(\\\"resolution\\\") {\\n-            if pubspec_object.extra[\\\"resolution\\\"] == \\\"workspace\\\" {\\n-                let mut current_dir = current_dir.as_ref();\\n-                loop {\\n-                    if current_dir == Path::new(\\\"/\\\") {\\n-                        return Err(io::Error::new(\\n-                            io::ErrorKind::NotFound,\\n-                            \\\"No parent directory\\\",\\n-                        ));\\n-                    }\\n-\\n-                    let parent_pubspec = match PubspecFile::read_pubspec_toml(current_dir) {\\n-                        Ok(pubspec) => pubspec,\\n-                        Err(_) => match PubspecFile::read_pubspec_yaml(current_dir) {\\n+                    PubspecFile::read_pubspec_yaml_lsp_test(&child_dir)?\\n+                } else {\\n+                    return Err(format!(\\n+                        \\\"Workspace member missing pubspec in {}\\\",\\n+                        child_dir.display()\\n+                    )\\n+                    .into());\\n+                };\\n+\\n+                map.insert(child_dir, child_pubspec);\\n+            }\\n+\\n+            logger!(Debug, \\\"{fn_name} Returning Workspace type\\\");\\n+            return Ok(DartProjectType::Workspace {\\n+                root: (dir, root_pubspec),\\n+                members: map,\\n+            });\\n+        }\\n+        logger!(Debug, \\\"{fn_name} Cleaning up the method\\\");\\n+        Ok(DartProjectType::Standard(dir.to_path_buf(), root_pubspec))\\n+    }\\n+\\n+    fn find_file_recursive(dir: &mut PathBuf) -> Result<Pubspec, Box<dyn std::error::Error>> {\\n+        let root_dir = PathBuf::from(\\\"/\\\");\\n+\\n+        // Detect and load root pubspec (TOML preferred if both exist).\\n+        while *dir != root_dir {\\n+            logger!(Debug, \\\"dir = {}\\\", dir.display());\\n+            let yaml_path = PubspecFile::pubspec_path(&dir);\\n+            let toml_path = PubspecFile::pubspec_toml_path(&dir);\\n+            if toml_path.exists() {\\n+                return PubspecFile::read_pubspec_toml(&dir).map_err(|err| err.into());\\n+            } else if yaml_path.exists() {\\n+                return PubspecFile::read_pubspec_yaml_lsp_test(&dir).map_err(|err| err.into());\\n+            };\\n+\\n+            dir.pop();\\n+        }\\n+\\n+        Err(\\\"Not a dart project\\\".into())\\n+    }\\n+\\n+    pub fn is_workspace_child(\\n+        pubspec_object: &Pubspec,\\n+        current_dir: impl AsRef<Path>,\\n+    ) -> Result<(Pubspec, &'static str, PathBuf), io::Error> {\\n+        if pubspec_object.extra.contains_key(\\\"resolution\\\") {\\n+            if pubspec_object.extra[\\\"resolution\\\"] == \\\"workspace\\\" {\\n+                let mut current_dir = current_dir.as_ref();\\n+                loop {\\n+                    if current_dir == Path::new(\\\"/\\\") {\\n+                        return Err(io::Error::new(\\n+                            io::ErrorKind::NotFound,\\n+                            \\\"No parent directory\\\",\\n+                        ));\\n+                    }\\n+\\n+                    let parent_pubspec = match PubspecFile::read_pubspec_toml(current_dir) {\\n+                        Ok(pubspec) => pubspec,\\n+                        Err(_) => match PubspecFile::read_pubspec_yaml_lsp_test(current_dir) {\",\"file\":\"bamboo_core/src/project_type.rs\",\"truncated\":false}],\"macro_reference_candidates\":{\"candidate_edits_added\":false,\"issue_count\":0,\"items\":[],\"review_note\":\"These are possible old-name tokens remaining in macro_rules! bodies. They are unverified and were not added to the rename edit.\",\"scan_complete\":true,\"scanned_files\":55,\"symbol\":\"read_pubspec_yaml\",\"truncated\":false},\"preview_id\":\"91b03a25-d976-4006-acb1-4be8881c209c\"}"
      }
    ],
    "isError": false
  }
}
````

### 22. `aimer_rust_preview_code_action`

**Purpose:** Preview the first returned code action; do not apply.  
**Elapsed:** 0.002s  
**Observed:** preview changed=true, files=1

**Arguments**

````json
{
  "action_id": "7de822b6-bb37-46f3-92b0-538ea78ec74e"
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 24,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_preview_code_action",
    "arguments": {
      "action_id": "7de822b6-bb37-46f3-92b0-538ea78ec74e"
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-24"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 24,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"changed\":true,\"files\":[{\"diff\":\"@@ line 26 @@\\n-        let content = fs::read_to_string(Self::pubspec_path(dir))?;\\n+        let read_to_string = fs::read_to_string(Self::pubspec_path(dir))?;\\n+        let content = read_to_string;\",\"file\":\"bamboo_core/src/file_reader.rs\",\"truncated\":false}],\"preview_id\":\"95df5eda-193d-4fe4-9133-57bc76b2d241\"}"
      }
    ],
    "isError": false
  }
}
````

### 23. `aimer_rust_signature_help`

**Purpose:** Signature help at an actual call.  
**Elapsed:** 0.506s  
**Observed:** signatures=1

**Arguments**

````json
{
  "file": "bamboo_core/src/bamboo_util.rs",
  "line": 24,
  "column": 48
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 25,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_signature_help",
    "arguments": {
      "file": "bamboo_core/src/bamboo_util.rs",
      "line": 24,
      "column": 48
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-25"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 25,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"activeParameter\":0,\"activeSignature\":0,\"signatures\":[{\"activeParameter\":0,\"documentation\":{\"kind\":\"markdown\",\"value\":\"Reads the `pubspec.yaml` file from the given directory and deserializes it into `PubspecObject`.\\n\\n# Errors\\nReturns an `io::Error` if the file cannot be read or if the YAML is invalid.\"},\"label\":\"fn read_pubspec_yaml(dir: &PathBuf) -> Result<Pubspec, Error>\",\"parameters\":[{\"label\":\"dir: &PathBuf\"}]}]}"
      }
    ],
    "isError": false
  }
}
````

### 24. `aimer_rust_type_definition`

**Purpose:** Type definition at Pubspec in the function return type.  
**Elapsed:** 0.508s  
**Observed:** results=1

**Arguments**

````json
{
  "file": "bamboo_core/src/file_reader.rs",
  "line": 25,
  "column": 68
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 26,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_type_definition",
    "arguments": {
      "file": "bamboo_core/src/file_reader.rs",
      "line": 25,
      "column": 68
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-26"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 26,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"results\":[{\"originSelectionRange\":{\"end\":{\"character\":73,\"line\":24},\"start\":{\"character\":66,\"line\":24}},\"target_file\":\"bamboo_core/src/pubspec.rs\",\"target_range\":{\"end\":{\"column\":2,\"line\":98},\"start\":{\"column\":1,\"line\":41}},\"target_selection_range\":{\"end\":{\"column\":19,\"line\":45},\"start\":{\"column\":12,\"line\":45}}}],\"truncated\":false}"
      }
    ],
    "isError": false
  }
}
````

### 25. `aimer_rust_apply_edit`

**Purpose:** Negative-path check only; no edit applied.  
**Elapsed:** 0.004s  
**Observed:** expected rejection of invalid preview ID

**Arguments**

````json
{
  "preview_id": "intentionally-invalid-preview-for-no-mutation"
}
````

**JSON-RPC request**

````json
{
  "jsonrpc": "2.0",
  "id": 27,
  "method": "tools/call",
  "params": {
    "name": "aimer_rust_apply_edit",
    "arguments": {
      "preview_id": "intentionally-invalid-preview-for-no-mutation"
    },
    "_meta": {
      "progressToken": "aimer-lsp-report-27"
    }
  }
}
````

**Progress notifications:** none.

**Raw MCP response**

````json
{
  "jsonrpc": "2.0",
  "id": 27,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"error\":\"unknown, expired, or already applied edit preview\"}"
      }
    ],
    "isError": false
  }
}
````

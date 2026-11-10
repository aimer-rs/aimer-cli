//! Agents that `aimer mcp --install` can register with, and how each one
//! stores its user-level MCP server configuration.

use std::path::{Path, PathBuf};

use serde_json::{Value as JsonValue, json};

/// Shape of the `aimer` server object inside a JSON configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EntryShape {
    /// `{ "command": ..., "args": [...] }`.
    Standard,
    /// `{ "type": "stdio", "command": ..., "args": [...] }`.
    Stdio,
    /// `{ "type": "local", "command": ..., "args": [...], "tools": ["*"] }`.
    CopilotLocal,
    /// `{ "type": "local", "command": [...], "enabled": true }`.
    OpenCode,
}

impl EntryShape {
    /// Fields written into the server object; other existing fields are kept.
    pub(super) fn fields(self, command: &str, args: &[String]) -> Vec<(&'static str, JsonValue)> {
        match self {
            Self::Standard => vec![("command", json!(command)), ("args", json!(args))],
            Self::Stdio => vec![
                ("type", json!("stdio")),
                ("command", json!(command)),
                ("args", json!(args)),
            ],
            Self::CopilotLocal => vec![
                ("type", json!("local")),
                ("command", json!(command)),
                ("args", json!(args)),
                ("tools", json!(["*"])),
            ],
            Self::OpenCode => {
                let mut argv = vec![command.to_string()];
                argv.extend(args.iter().cloned());
                vec![
                    ("type", json!("local")),
                    ("command", json!(argv)),
                    ("enabled", json!(true)),
                ]
            }
        }
    }
}

/// File format of an agent's MCP configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConfigKind {
    /// TOML with an `mcp_servers` table.
    Toml,
    /// JSON with `servers_key` holding an object of servers.
    Json {
        servers_key: &'static str,
        entry: EntryShape,
    },
}

/// One agent with a user-level MCP configuration file.
pub(super) struct McpAgent {
    /// Canonical identifier accepted after `--install`.
    pub name: &'static str,
    /// Other identifiers accepted for the same agent.
    pub aliases: &'static [&'static str],
    /// Human-readable product name.
    pub label: &'static str,
    /// Configuration file relative to the home directory.
    pub config: &'static str,
    pub kind: ConfigKind,
}

impl McpAgent {
    /// Absolute configuration path; Codex honors `CODEX_HOME` when requested.
    pub(super) fn config_path(&self, home: &Path, honor_codex_home: bool) -> PathBuf {
        if self.name == "codex" && honor_codex_home {
            if let Some(root) = std::env::var_os("CODEX_HOME") {
                return PathBuf::from(root).join("config.toml");
            }
        }
        home.join(self.config)
    }
}

const fn json_agent(
    name: &'static str,
    label: &'static str,
    config: &'static str,
    servers_key: &'static str,
    entry: EntryShape,
) -> McpAgent {
    McpAgent {
        name,
        aliases: &[],
        label,
        config,
        kind: ConfigKind::Json { servers_key, entry },
    }
}

const fn standard(name: &'static str, label: &'static str, config: &'static str) -> McpAgent {
    json_agent(name, label, config, "mcpServers", EntryShape::Standard)
}

pub(super) const AGENTS: &[McpAgent] = &[
    McpAgent {
        name: "codex",
        aliases: &[],
        label: "Codex",
        config: ".codex/config.toml",
        kind: ConfigKind::Toml,
    },
    McpAgent {
        name: "claude",
        aliases: &["claude-code"],
        label: "Claude Code",
        config: ".claude.json",
        kind: ConfigKind::Json {
            servers_key: "mcpServers",
            entry: EntryShape::Standard,
        },
    },
    standard("cursor", "Cursor", ".cursor/mcp.json"),
    standard("gemini-cli", "Gemini CLI", ".gemini/settings.json"),
    standard("windsurf", "Windsurf", ".codeium/windsurf/mcp_config.json"),
    standard("qwen-code", "Qwen Code", ".qwen/settings.json"),
    standard("kiro-cli", "Kiro CLI", ".kiro/settings/mcp.json"),
    standard("junie", "Junie", ".junie/mcp/mcp.json"),
    standard("iflow-cli", "iFlow CLI", ".iflow/settings.json"),
    standard("augment", "Augment", ".augment/settings.json"),
    standard("codebuddy", "CodeBuddy", ".codebuddy/mcp.json"),
    standard("kimi-code-cli", "Kimi Code CLI", ".kimi/mcp.json"),
    json_agent(
        "droid",
        "Droid",
        ".factory/mcp.json",
        "mcpServers",
        EntryShape::Stdio,
    ),
    json_agent(
        "github-copilot",
        "GitHub Copilot CLI",
        ".copilot/mcp-config.json",
        "mcpServers",
        EntryShape::CopilotLocal,
    ),
    json_agent(
        "opencode",
        "OpenCode",
        ".config/opencode/opencode.json",
        "mcp",
        EntryShape::OpenCode,
    ),
    json_agent(
        "crush",
        "Crush",
        ".config/crush/crush.json",
        "mcp",
        EntryShape::Stdio,
    ),
    json_agent(
        "amp",
        "Amp",
        ".config/amp/settings.json",
        "amp.mcpServers",
        EntryShape::Standard,
    ),
];

/// Find an agent by canonical name or alias, ignoring case and whitespace.
pub(super) fn find(value: &str) -> Option<&'static McpAgent> {
    let value = value.trim().to_ascii_lowercase();
    AGENTS
        .iter()
        .find(|agent| agent.name == value || agent.aliases.contains(&value.as_str()))
}

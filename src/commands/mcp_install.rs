use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use toml::{Table as TomlTable, Value as TomlValue};
use uuid::Uuid;

use super::agent_list::{self, AgentRow};
use super::mcp_agents::{self, ConfigKind, McpAgent};

/// Describes one agent configuration updated by an installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallReport {
    /// Canonical agent name.
    pub agent: String,
    /// Configuration file updated for this agent.
    pub path: PathBuf,
    /// Whether the file content changed.
    pub changed: bool,
}

/// Text format used when printing the MCP entry instead of installing it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigFormat {
    /// The `mcpServers` object used by Claude Code and Cursor.
    Json,
    /// The `mcp_servers` table used by Codex.
    Toml,
}

/// Render Aimer's global stdio MCP entry without writing any agent configuration.
///
/// The entry is not bound to a project: the running MCP adapter discovers the
/// nearest `Aimer.toml` or `Cargo.toml` from its working directory.
pub fn render_config(format: ConfigFormat) -> Result<String> {
    let executable = std::env::current_exe()
        .context("cannot determine the aimer executable")?
        .canonicalize()
        .context("cannot resolve the aimer executable")?;
    render_config_at(format, &executable)
}

fn render_config_at(format: ConfigFormat, executable: &Path) -> Result<String> {
    let args = command_args();
    let command = executable.display().to_string();
    match format {
        ConfigFormat::Json => {
            let document = json!({
                "mcpServers": { "aimer": { "command": command, "args": args } }
            });
            serde_json::to_string_pretty(&document).context("serializing MCP configuration")
        }
        ConfigFormat::Toml => {
            let mut server = TomlTable::new();
            server.insert("command".to_string(), TomlValue::String(command));
            server.insert(
                "args".to_string(),
                TomlValue::Array(args.into_iter().map(TomlValue::String).collect()),
            );
            let mut servers = TomlTable::new();
            servers.insert("aimer".to_string(), TomlValue::Table(server));
            let mut root = TomlTable::new();
            root.insert("mcp_servers".to_string(), TomlValue::Table(servers));
            toml::to_string_pretty(&TomlValue::Table(root)).context("serializing MCP configuration")
        }
    }
}

/// Install Aimer's global stdio MCP entry in the requested agent configurations.
pub fn install(agents: &[String]) -> Result<Vec<InstallReport>> {
    let home = dirs::home_dir().context("cannot determine the user home directory")?;
    let executable = std::env::current_exe()
        .context("cannot determine the aimer executable")?
        .canonicalize()
        .context("cannot resolve the aimer executable")?;
    install_at(agents, &home, &executable, true)
}

fn install_at(
    agents: &[String],
    home: &Path,
    executable: &Path,
    honor_codex_home: bool,
) -> Result<Vec<InstallReport>> {
    if agents.is_empty() {
        bail!("at least one MCP agent is required after --install");
    }

    let agents = agents
        .iter()
        .map(|agent| parse_agent(agent))
        .collect::<Result<Vec<_>>>()?;
    let args = command_args();
    let command = executable.display().to_string();

    let mut reports = Vec::with_capacity(agents.len());
    for agent in agents {
        let path = agent.config_path(home, honor_codex_home);
        let changed = match agent.kind {
            ConfigKind::Toml => install_toml(&path, executable, &args)?,
            ConfigKind::Json { servers_key, entry } => {
                install_json(&path, servers_key, &entry.fields(&command, &args))?
            }
        };
        reports.push(InstallReport {
            agent: agent.name.to_string(),
            path,
            changed,
        });
    }
    Ok(reports)
}

fn command_args() -> Vec<String> {
    vec!["mcp".to_string()]
}

/// List the agents accepted by `aimer mcp --install`.
pub fn list_agents() -> String {
    let rows = mcp_agents::AGENTS
        .iter()
        .map(|agent| {
            AgentRow::new(
                agent.name,
                agent.aliases,
                agent.label,
                format!("~/{}", agent.config),
            )
        })
        .collect::<Vec<_>>();
    agent_list::render(
        "Agents supported by `aimer mcp --install`:",
        "aimer mcp --install <AGENT>[,<AGENT>...]",
        &rows,
    )
}

fn parse_agent(value: &str) -> Result<&'static McpAgent> {
    mcp_agents::find(value).ok_or_else(|| {
        anyhow!(
            "unsupported MCP agent '{}'; run `aimer mcp --install` without an agent to list the supported agents",
            value.trim().to_ascii_lowercase()
        )
    })
}

pub(crate) fn discover_project_root(start: &Path) -> Result<PathBuf> {
    let canonical = start
        .canonicalize()
        .with_context(|| format!("resolving project path '{}'", start.display()))?;
    let directory = if canonical.is_dir() {
        canonical.as_path()
    } else {
        canonical
            .parent()
            .ok_or_else(|| anyhow!("project path '{}' has no parent", start.display()))?
    };

    for candidate in directory.ancestors() {
        if candidate.join(crate::config::MANIFEST_FILE).is_file()
            || candidate.join("Cargo.toml").is_file()
        {
            return Ok(candidate.to_path_buf());
        }
    }

    bail!(
        "no {} or Cargo.toml found from '{}'; run this command from an Aimer or Rust project, or pass --project",
        crate::config::MANIFEST_FILE,
        start.display()
    )
}

fn install_toml(path: &Path, executable: &Path, args: &[String]) -> Result<bool> {
    let mut document = if path.exists() {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("reading MCP configuration '{}'", path.display()))?;
        toml::from_str::<TomlValue>(&contents)
            .with_context(|| format!("parsing MCP configuration '{}'", path.display()))?
    } else {
        TomlValue::Table(TomlTable::new())
    };

    let root = document.as_table_mut().ok_or_else(|| {
        anyhow!(
            "MCP configuration '{}' must contain a TOML table",
            path.display()
        )
    })?;
    let servers = root
        .entry("mcp_servers")
        .or_insert_with(|| TomlValue::Table(TomlTable::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow!("'mcp_servers' in '{}' must be a TOML table", path.display()))?;
    let server = servers
        .entry("aimer")
        .or_insert_with(|| TomlValue::Table(TomlTable::new()))
        .as_table_mut()
        .ok_or_else(|| {
            anyhow!(
                "'mcp_servers.aimer' in '{}' must be a TOML table",
                path.display()
            )
        })?;

    server.insert(
        "command".to_string(),
        TomlValue::String(executable.display().to_string()),
    );
    server.insert(
        "args".to_string(),
        TomlValue::Array(args.iter().cloned().map(TomlValue::String).collect()),
    );

    let contents = toml::to_string_pretty(&document)
        .with_context(|| format!("serializing MCP configuration '{}'", path.display()))?;
    write_if_changed(path, contents.as_bytes())
}

fn install_json(
    path: &Path,
    servers_key: &str,
    fields: &[(&'static str, JsonValue)],
) -> Result<bool> {
    let mut document = if path.exists() {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("reading MCP configuration '{}'", path.display()))?;
        serde_json::from_str::<JsonValue>(&contents)
            .with_context(|| format!("parsing MCP configuration '{}'", path.display()))?
    } else {
        JsonValue::Object(JsonMap::new())
    };

    let root = document.as_object_mut().ok_or_else(|| {
        anyhow!(
            "MCP configuration '{}' must contain a JSON object",
            path.display()
        )
    })?;
    let servers = root
        .entry(servers_key)
        .or_insert_with(|| JsonValue::Object(JsonMap::new()))
        .as_object_mut()
        .ok_or_else(|| {
            anyhow!(
                "'{servers_key}' in '{}' must be a JSON object",
                path.display()
            )
        })?;
    let server = servers
        .entry("aimer")
        .or_insert_with(|| JsonValue::Object(JsonMap::new()))
        .as_object_mut()
        .ok_or_else(|| {
            anyhow!(
                "'{servers_key}.aimer' in '{}' must be a JSON object",
                path.display()
            )
        })?;

    for (key, value) in fields {
        server.insert((*key).to_string(), value.clone());
    }

    let contents = serde_json::to_vec_pretty(&document)
        .with_context(|| format!("serializing MCP configuration '{}'", path.display()))?;
    write_if_changed(path, &contents)
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<bool> {
    match fs::read(path) {
        Ok(existing) if existing == contents => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading MCP configuration '{}'", path.display()));
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "creating MCP configuration directory '{}'",
                parent.display()
            )
        })?;
    }

    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mcp"),
        Uuid::new_v4()
    ));
    fs::write(&temporary, contents).with_context(|| {
        format!(
            "writing temporary MCP configuration '{}'",
            temporary.display()
        )
    })?;

    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temporary, metadata.permissions()).with_context(|| {
            format!(
                "preserving permissions for MCP configuration '{}'",
                path.display()
            )
        })?;
    } else {
        #[cfg(unix)]
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("securing new MCP configuration '{}'", path.display()))?;
    }

    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error)
            .with_context(|| format!("installing MCP configuration '{}'", path.display()));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn global_codex_install_uses_runtime_project_discovery() {
        let home = tempfile::tempdir().unwrap();
        let executable = Path::new("/opt/aimer/bin/aimer");

        let reports = install_in(&["codex".to_string()], home.path(), executable).unwrap();

        assert_eq!(reports.len(), 1);
        let config: toml::Value =
            toml::from_str(&fs::read_to_string(home.path().join(".codex/config.toml")).unwrap())
                .unwrap();
        assert_eq!(
            config["mcp_servers"]["aimer"]["command"],
            TomlValue::String(executable.display().to_string())
        );
        assert_eq!(
            config["mcp_servers"]["aimer"]["args"],
            TomlValue::Array(vec![TomlValue::String("mcp".to_string())])
        );
    }

    #[test]
    fn install_preserves_other_servers_and_is_idempotent() {
        let home = tempfile::tempdir().unwrap();
        let config_dir = home.path().join(".codex");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            "[mcp_servers.keep]\ncommand = \"keep\"\nargs = [\"--keep\"]\n\n[mcp_servers.aimer]\nstartup_timeout_sec = 120\n",
        )
        .unwrap();

        let first = install_in(
            &["codex".to_string()],
            home.path(),
            Path::new("/opt/aimer/bin/aimer"),
        )
        .unwrap();
        let second = install_in(
            &["codex".to_string()],
            home.path(),
            Path::new("/opt/aimer/bin/aimer"),
        )
        .unwrap();

        assert!(first[0].changed);
        assert!(!second[0].changed);
        let config: toml::Value =
            toml::from_str(&fs::read_to_string(config_dir.join("config.toml")).unwrap()).unwrap();
        assert_eq!(
            config["mcp_servers"]["keep"]["command"],
            TomlValue::String("keep".to_string())
        );
        assert_eq!(
            config["mcp_servers"]["aimer"]["startup_timeout_sec"],
            TomlValue::Integer(120)
        );
    }

    #[test]
    fn json_agents_receive_the_same_global_stdio_entry() {
        let home = tempfile::tempdir().unwrap();
        let reports = install_in(
            &["claude".to_string(), "cursor".to_string()],
            home.path(),
            Path::new("/opt/aimer/bin/aimer"),
        )
        .unwrap();

        assert_eq!(reports.len(), 2);
        for path in [
            home.path().join(".claude.json"),
            home.path().join(".cursor/mcp.json"),
        ] {
            let config: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(
                config["mcpServers"]["aimer"]["command"],
                "/opt/aimer/bin/aimer"
            );
            assert_eq!(
                config["mcpServers"]["aimer"]["args"],
                serde_json::json!(["mcp"])
            );
        }
    }

    #[test]
    fn rendered_json_matches_the_installed_entry_without_writing() {
        let executable = Path::new("/opt/aimer/bin/aimer");

        let text = render_config_at(ConfigFormat::Json, executable).unwrap();

        let config: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            config["mcpServers"]["aimer"],
            serde_json::json!({ "command": "/opt/aimer/bin/aimer", "args": ["mcp"] })
        );
    }

    #[test]
    fn rendered_toml_is_not_bound_to_a_project() {
        let text = render_config_at(ConfigFormat::Toml, Path::new("/opt/aimer/bin/aimer")).unwrap();

        let config: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(
            config["mcp_servers"]["aimer"]["command"],
            TomlValue::String("/opt/aimer/bin/aimer".to_string())
        );
        assert_eq!(
            config["mcp_servers"]["aimer"]["args"],
            TomlValue::Array(vec![TomlValue::String("mcp".to_string())])
        );
    }

    #[test]
    fn agents_use_their_own_config_file_and_entry_shape() {
        let home = tempfile::tempdir().unwrap();
        let agents = [
            "gemini-cli",
            "opencode",
            "github-copilot",
            "amp",
            "droid",
            "junie",
        ]
        .map(str::to_string)
        .to_vec();

        let reports = install_in(&agents, home.path(), Path::new("/opt/aimer/bin/aimer")).unwrap();

        assert_eq!(reports.len(), 6);
        let read = |relative: &str| -> serde_json::Value {
            serde_json::from_str(&fs::read_to_string(home.path().join(relative)).unwrap()).unwrap()
        };
        let standard = serde_json::json!({ "command": "/opt/aimer/bin/aimer", "args": ["mcp"] });
        assert_eq!(
            read(".gemini/settings.json")["mcpServers"]["aimer"],
            standard
        );
        assert_eq!(read(".junie/mcp/mcp.json")["mcpServers"]["aimer"], standard);
        assert_eq!(
            read(".config/amp/settings.json")["amp.mcpServers"]["aimer"],
            standard
        );
        assert_eq!(
            read(".config/opencode/opencode.json")["mcp"]["aimer"],
            serde_json::json!({
                "type": "local",
                "command": ["/opt/aimer/bin/aimer", "mcp"],
                "enabled": true
            })
        );
        assert_eq!(
            read(".copilot/mcp-config.json")["mcpServers"]["aimer"],
            serde_json::json!({
                "type": "local",
                "command": "/opt/aimer/bin/aimer",
                "args": ["mcp"],
                "tools": ["*"]
            })
        );
        assert_eq!(
            read(".factory/mcp.json")["mcpServers"]["aimer"]["type"],
            "stdio"
        );
    }

    #[test]
    fn json_install_keeps_other_settings_and_servers() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".gemini/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"theme":"dark","mcpServers":{"keep":{"command":"keep"}}}"#,
        )
        .unwrap();

        install_in(
            &["gemini-cli".to_string()],
            home.path(),
            Path::new("/opt/aimer/bin/aimer"),
        )
        .unwrap();

        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(config["theme"], "dark");
        assert_eq!(config["mcpServers"]["keep"]["command"], "keep");
        assert_eq!(
            config["mcpServers"]["aimer"]["args"],
            serde_json::json!(["mcp"])
        );
    }

    #[test]
    fn agent_names_and_aliases_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for agent in mcp_agents::AGENTS {
            for name in std::iter::once(&agent.name).chain(agent.aliases) {
                assert!(seen.insert(*name), "duplicate agent identifier {name}");
            }
        }
    }

    #[test]
    fn listing_shows_every_agent_with_its_config_path() {
        let text = list_agents();

        for agent in mcp_agents::AGENTS {
            assert!(text.contains(agent.name), "{} missing", agent.name);
        }
        assert!(text.contains("claude (claude-code)"));
        assert!(text.contains("~/.cursor/mcp.json"));
    }

    #[test]
    fn unsupported_agent_is_rejected_before_writing() {
        let home = tempfile::tempdir().unwrap();
        let error = install_in(
            &["unknown-agent".to_string()],
            home.path(),
            Path::new("/opt/aimer/bin/aimer"),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("unsupported MCP agent 'unknown-agent'"));
        assert!(!home.path().join(".codex/config.toml").exists());
    }

    fn install_in(agents: &[String], home: &Path, executable: &Path) -> Result<Vec<InstallReport>> {
        install_at(agents, home, executable, false)
    }
}

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

const AIMER_RUST_SKILL: &str = include_str!("../../aimer-rust/SKILL.md");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillInstallReport {
    pub agent: String,
    pub path: PathBuf,
    pub changed: bool,
}

/// Install the maintained Aimer Rust skill into user-level agent skill folders.
pub fn install(agents: &[String]) -> Result<Vec<SkillInstallReport>> {
    let home = dirs::home_dir().context("cannot determine the user home directory")?;
    install_at(agents, &home)
}

fn install_at(agents: &[String], home: &Path) -> Result<Vec<SkillInstallReport>> {
    anyhow::ensure!(
        !agents.is_empty(),
        "at least one agent is required after --install"
    );
    agents
        .iter()
        .map(|agent| {
            let agent = Agent::parse(agent)?;
            let path = agent.skill_path(home);
            let changed = match fs::read_to_string(&path) {
                Ok(contents) if contents == AIMER_RUST_SKILL => false,
                Ok(_) => write_skill(&path)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => write_skill(&path)?,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("reading existing skill file '{}'", path.display())
                    });
                }
            };
            Ok(SkillInstallReport {
                agent: agent.name().to_owned(),
                path,
                changed,
            })
        })
        .collect()
}

fn write_skill(path: &Path) -> Result<bool> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating skill directory '{}'", parent.display()))?;
    }
    fs::write(path, AIMER_RUST_SKILL)
        .with_context(|| format!("writing Aimer Rust skill to '{}'", path.display()))?;
    Ok(true)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Agent {
    Codex,
    Claude,
    Cursor,
}

impl Agent {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" => Ok(Self::Claude),
            "cursor" => Ok(Self::Cursor),
            other => Err(anyhow!(
                "unsupported skill agent '{other}'; supported agents: codex, claude, cursor"
            )),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
        }
    }

    fn skill_path(self, home: &Path) -> PathBuf {
        let root = match self {
            Self::Codex => home.join(".agents/skills"),
            Self::Claude => home.join(".claude/skills"),
            Self::Cursor => home.join(".cursor/skills"),
        };
        root.join("aimer-rust/SKILL.md")
    }
}

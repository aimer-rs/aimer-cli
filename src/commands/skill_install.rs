use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use super::agent_list::{self, AgentRow};
use super::skill_agents::{self, SkillAgent};

const AIMER_RUST_SKILL: &str = include_str!("../../aimer-rust/SKILL.md");
const AIMER_APP_SKILL: &str = include_str!("../../aimer-app/SKILL.md");

/// A skill shipped inside the binary, installed as `<skills dir>/<name>/SKILL.md`.
struct Skill {
    name: &'static str,
    contents: &'static str,
}

const SKILLS: &[Skill] = &[
    Skill {
        name: "aimer-rust",
        contents: AIMER_RUST_SKILL,
    },
    Skill {
        name: "aimer-app",
        contents: AIMER_APP_SKILL,
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillInstallReport {
    pub agent: String,
    pub skill: &'static str,
    pub path: PathBuf,
    pub changed: bool,
}

/// List the agents accepted by `aimer skill --install`.
pub fn list_agents() -> String {
    let rows = skill_agents::AGENTS
        .iter()
        .map(|agent| {
            AgentRow::new(
                agent.name,
                agent.aliases,
                agent.label,
                format!("~/{}/{{{}}}/SKILL.md", agent.dir, skill_names()),
            )
        })
        .collect::<Vec<_>>();
    agent_list::render(
        "Agents supported by `aimer skill --install`:",
        "aimer skill --install <AGENT>[,<AGENT>...]",
        &rows,
    )
}

fn skill_names() -> String {
    SKILLS
        .iter()
        .map(|skill| skill.name)
        .collect::<Vec<_>>()
        .join(",")
}

/// Install the maintained Aimer skills into user-level agent skill folders.
pub fn install(agents: &[String]) -> Result<Vec<SkillInstallReport>> {
    let home = dirs::home_dir().context("cannot determine the user home directory")?;
    install_at(agents, &home)
}

fn install_at(agents: &[String], home: &Path) -> Result<Vec<SkillInstallReport>> {
    anyhow::ensure!(
        !agents.is_empty(),
        "at least one agent is required after --install"
    );
    let agents = agents
        .iter()
        .map(|agent| parse_agent(agent))
        .collect::<Result<Vec<_>>>()?;
    let mut reports = Vec::new();
    for agent in agents {
        for skill in SKILLS {
            let path = skill_path(agent, skill, home);
            let changed = match fs::read_to_string(&path) {
                Ok(contents) if contents == skill.contents => false,
                Ok(_) => write_skill(&path, skill)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    write_skill(&path, skill)?
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("reading existing skill file '{}'", path.display())
                    });
                }
            };
            reports.push(SkillInstallReport {
                agent: agent.name.to_owned(),
                skill: skill.name,
                path,
                changed,
            });
        }
    }
    Ok(reports)
}

fn write_skill(path: &Path, skill: &Skill) -> Result<bool> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating skill directory '{}'", parent.display()))?;
    }
    fs::write(path, skill.contents).with_context(|| {
        format!("writing skill '{}' to '{}'", skill.name, path.display())
    })?;
    Ok(true)
}

fn parse_agent(value: &str) -> Result<&'static SkillAgent> {
    skill_agents::find(value).ok_or_else(|| {
        anyhow!(
            "unsupported skill agent '{}'; run `aimer skill --install` without an agent to list the supported agents",
            value.trim().to_ascii_lowercase()
        )
    })
}

fn skill_path(agent: &SkillAgent, skill: &Skill, home: &Path) -> PathBuf {
    home.join(agent.dir).join(skill.name).join("SKILL.md")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_into_each_agents_global_skills_directory() {
        let home = tempfile::tempdir().unwrap();
        let agents = ["claude-code", "gemini-cli", "windsurf", "codex"]
            .map(str::to_string)
            .to_vec();

        let reports = install_at(&agents, home.path()).unwrap();

        assert_eq!(reports.len(), 8);
        assert_eq!(
            reports
                .iter()
                .map(|r| (r.agent.as_str(), r.skill))
                .collect::<Vec<_>>(),
            [
                ("claude", "aimer-rust"),
                ("claude", "aimer-app"),
                ("gemini-cli", "aimer-rust"),
                ("gemini-cli", "aimer-app"),
                ("windsurf", "aimer-rust"),
                ("windsurf", "aimer-app"),
                ("codex", "aimer-rust"),
                ("codex", "aimer-app"),
            ]
        );
        for relative in [
            ".claude/skills",
            ".gemini/skills",
            ".codeium/windsurf/skills",
            ".agents/skills",
        ] {
            let root = home.path().join(relative);
            assert_eq!(
                fs::read_to_string(root.join("aimer-rust/SKILL.md")).unwrap(),
                AIMER_RUST_SKILL
            );
            assert_eq!(
                fs::read_to_string(root.join("aimer-app/SKILL.md")).unwrap(),
                AIMER_APP_SKILL
            );
        }
        assert!(
            install_at(&agents, home.path())
                .unwrap()
                .iter()
                .all(|r| !r.changed)
        );
    }

    #[test]
    fn unknown_agent_is_rejected_before_writing() {
        let home = tempfile::tempdir().unwrap();
        let agents = vec!["cursor".to_string(), "nope".to_string()];

        let error = install_at(&agents, home.path()).unwrap_err().to_string();

        assert!(error.contains("unsupported skill agent 'nope'"));
        assert!(!home.path().join(".cursor").exists());
    }

    #[test]
    fn agent_names_and_aliases_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for agent in skill_agents::AGENTS {
            for name in std::iter::once(&agent.name).chain(agent.aliases) {
                assert!(seen.insert(*name), "duplicate agent identifier {name}");
            }
        }
    }

    #[test]
    fn listing_mentions_every_agent() {
        let text = list_agents();

        for agent in skill_agents::AGENTS {
            assert!(text.contains(agent.name), "{} missing", agent.name);
        }
        assert!(text.contains("~/.claude/skills/{aimer-rust,aimer-app}/SKILL.md"));
    }

    #[test]
    fn bundled_skills_declare_their_install_names() {
        for skill in SKILLS {
            let header = format!("---\nname: {}\n", skill.name);
            assert!(skill.contents.starts_with(&header), "{}", skill.name);
        }
    }
}

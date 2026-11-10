//! Agents that can receive Aimer's Rust skill, with their user-level skills
//! directories (relative to the home directory).
//!
//! Source: <https://github.com/vercel-labs/skills/blob/26dc8e4755a11fea22ed788256c3db50001b013d/docs/agents.md>.
//! Agents documented as project-only have no global directory and are omitted.

/// One agent with a user-level skills directory.
pub(super) struct SkillAgent {
    /// Canonical identifier accepted after `--install`.
    pub name: &'static str,
    /// Other identifiers accepted for the same agent.
    pub aliases: &'static [&'static str],
    /// Human-readable product name.
    pub label: &'static str,
    /// Global skills directory relative to the home directory.
    pub dir: &'static str,
}

const fn agent(name: &'static str, label: &'static str, dir: &'static str) -> SkillAgent {
    SkillAgent {
        name,
        aliases: &[],
        label,
        dir,
    }
}

const fn agent_with_aliases(
    name: &'static str,
    aliases: &'static [&'static str],
    label: &'static str,
    dir: &'static str,
) -> SkillAgent {
    SkillAgent {
        name,
        aliases,
        label,
        dir,
    }
}

pub(super) const AGENTS: &[SkillAgent] = &[
    agent("aider-desk", "AiderDesk", ".aider-desk/skills"),
    agent("amp", "Amp", ".config/agents/skills"),
    agent("replit", "Replit", ".config/agents/skills"),
    agent("universal", "Universal", ".config/agents/skills"),
    agent("antigravity", "Antigravity", ".gemini/config/skills"),
    agent(
        "antigravity-cli",
        "Antigravity CLI",
        ".gemini/config/skills",
    ),
    agent("astrbot", "AstrBot", ".astrbot/data/skills"),
    agent("autohand-code", "Autohand Code CLI", ".autohand/skills"),
    agent("augment", "Augment", ".augment/skills"),
    agent("bob", "IBM Bob", ".bob/skills"),
    agent_with_aliases("claude", &["claude-code"], "Claude Code", ".claude/skills"),
    agent("openclaw", "OpenClaw", ".openclaw/skills"),
    agent("cline", "Cline", ".agents/skills"),
    agent("codex", "Codex", ".agents/skills"),
    agent("dexto", "Dexto", ".agents/skills"),
    agent("fx", "fx", ".agents/skills"),
    agent("kimi-code-cli", "Kimi Code CLI", ".agents/skills"),
    agent("loaf", "Loaf", ".agents/skills"),
    agent("pi", "Pi", ".agents/skills"),
    agent("sarvam-code", "Sarvam Code", ".agents/skills"),
    agent("warp", "Warp", ".agents/skills"),
    agent("zed", "Zed", ".agents/skills"),
    agent("codearts-agent", "CodeArts Agent", ".codeartsdoer/skills"),
    agent("codebuddy", "CodeBuddy", ".codebuddy/skills"),
    agent("codemaker", "Codemaker", ".codemaker/skills"),
    agent("codestudio", "Code Studio", ".codestudio/skills"),
    agent("command-code", "Command Code", ".commandcode/skills"),
    agent("continue", "Continue", ".continue/skills"),
    agent("cortex", "Cortex Code", ".snowflake/cortex/skills"),
    agent("crush", "Crush", ".config/crush/skills"),
    agent("cursor", "Cursor", ".cursor/skills"),
    agent("deepagents", "Deep Agents", ".deepagents/agent/skills"),
    agent("devin", "Devin for Terminal", ".config/devin/skills"),
    agent("droid", "Droid", ".factory/skills"),
    agent("firebender", "Firebender", ".firebender/skills"),
    agent("forgecode", "ForgeCode", ".forge/skills"),
    agent("gemini-cli", "Gemini CLI", ".gemini/skills"),
    agent("github-copilot", "GitHub Copilot", ".copilot/skills"),
    agent("goose", "Goose", ".config/goose/skills"),
    agent("grok", "Grok Build", ".grok/skills"),
    agent("hermes-agent", "Hermes Agent", ".hermes/skills"),
    agent("inference-sh", "inference.sh", ".inferencesh/skills"),
    agent("jazz", "Jazz", ".jazz/skills"),
    agent("junie", "Junie", ".junie/skills"),
    agent("iflow-cli", "iFlow CLI", ".iflow/skills"),
    agent("kilo", "Kilo Code", ".kilo/skills"),
    agent("kimchi", "Kimchi", ".config/kimchi/harness/skills"),
    agent("kiro-cli", "Kiro CLI", ".kiro/skills"),
    agent("kode", "Kode", ".kode/skills"),
    agent("lingma", "Lingma", ".lingma/skills"),
    agent("mcpjam", "MCPJam", ".mcpjam/skills"),
    agent("minimax-code", "MiniMax Code", ".minimax/skills"),
    agent("mistral-vibe", "Mistral Vibe", ".vibe/skills"),
    agent("moxby", "Moxby", ".moxby/skills"),
    agent("mux", "Mux", ".mux/skills"),
    agent("opencode", "OpenCode", ".config/opencode/skills"),
    agent("openhands", "OpenHands", ".openhands/skills"),
    agent("ona", "Ona", ".ona/skills"),
    agent(
        "posit-assistant",
        "Posit Assistant",
        ".posit/assistant/skills",
    ),
    agent("qoder", "Qoder", ".qoder/skills"),
    agent("qoder-cn", "Qoder CN", ".qoder-cn/skills"),
    agent("qwen-code", "Qwen Code", ".qwen/skills"),
    agent("reasonix", "Reasonix", ".reasonix/skills"),
    agent("rovodev", "Rovo Dev", ".rovodev/skills"),
    agent("roo", "Roo Code", ".roo/skills"),
    agent("tabnine-cli", "Tabnine CLI", ".tabnine/agent/skills"),
    agent("terramind", "Terramind", ".terramind/skills"),
    agent("tinycloud", "Tinycloud", ".tinycloud/skills"),
    agent("trae", "Trae", ".trae/skills"),
    agent("trae-cn", "Trae CN", ".trae-cn/skills"),
    agent("windsurf", "Windsurf", ".codeium/windsurf/skills"),
    agent("zcode", "ZCode", ".zcode/skills"),
    agent("zencoder", "Zencoder", ".zencoder/skills"),
    agent("zenflow", "Zenflow", ".zencoder/skills"),
    agent("neovate", "Neovate", ".neovate/skills"),
    agent("pochi", "Pochi", ".pochi/skills"),
    agent("adal", "AdaL", ".adal/skills"),
];

/// Find an agent by canonical name or alias, ignoring case and whitespace.
pub(super) fn find(value: &str) -> Option<&'static SkillAgent> {
    let value = value.trim().to_ascii_lowercase();
    AGENTS
        .iter()
        .find(|agent| agent.name == value || agent.aliases.contains(&value.as_str()))
}

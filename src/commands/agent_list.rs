//! Shared plain-text rendering of the agents accepted by `--install`.

/// One agent row: identifier (with aliases), display name, and target location.
pub(crate) struct AgentRow {
    pub id: String,
    pub label: &'static str,
    pub location: String,
}

impl AgentRow {
    pub(crate) fn new(
        name: &str,
        aliases: &[&str],
        label: &'static str,
        location: impl Into<String>,
    ) -> Self {
        let id = if aliases.is_empty() {
            name.to_string()
        } else {
            format!("{name} ({})", aliases.join(", "))
        };
        Self {
            id,
            label,
            location: location.into(),
        }
    }
}

/// Render an aligned table below a heading and a usage line.
pub(crate) fn render(heading: &str, usage: &str, rows: &[AgentRow]) -> String {
    let id_width = rows.iter().map(|row| row.id.len()).max().unwrap_or(0);
    let label_width = rows.iter().map(|row| row.label.len()).max().unwrap_or(0);
    let mut text = format!("{heading}\n\n");
    for row in rows {
        text.push_str(&format!(
            "  {:<id_width$}  {:<label_width$}  {}\n",
            row.id, row.label, row.location
        ));
    }
    text.push_str(&format!("\nUsage: {usage}\n"));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_aligned_and_show_aliases() {
        let rows = [
            AgentRow::new("claude", &["claude-code"], "Claude Code", "~/.claude.json"),
            AgentRow::new("cursor", &[], "Cursor", "~/.cursor/mcp.json"),
        ];

        let text = render("Agents:", "aimer x --install <AGENT>", &rows);

        assert!(text.starts_with("Agents:\n\n"));
        assert!(text.contains("  claude (claude-code)  Claude Code  ~/.claude.json\n"));
        assert!(text.contains("  cursor                Cursor       ~/.cursor/mcp.json\n"));
        assert!(text.ends_with("\nUsage: aimer x --install <AGENT>\n"));
    }
}

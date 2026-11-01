//! Size limits that keep tool output from flooding an agent's context. They
//! apply to compact output only; full output is never trimmed.

/// Longest single text field kept as is.
const MAX_TEXT_CHARS: usize = 4_000;
const TEXT_HEAD_CHARS: usize = 2_500;
const TEXT_TAIL_CHARS: usize = 1_000;
/// Most items kept from one list; the rest are counted, not shown.
pub(super) const MAX_LIST_ITEMS: usize = 50;
/// Largest rendered response.
const MAX_OUTPUT_CHARS: usize = 24_000;

/// Strip terminal colour codes and trailing blanks, collapse runs of blank
/// lines, and shorten very long text to its head and tail.
pub(super) fn clean_text(text: &str) -> String {
    let plain = strip_ansi(text);
    let mut lines = Vec::new();
    let mut blank_run = 0;
    for line in plain.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        lines.push(line);
    }
    shorten(&lines.join("\n"), "use --full-output for the rest")
}

/// Limit the rendered response, keeping the start and noting what was cut.
pub(super) fn cap_output(markdown: String) -> String {
    let total = markdown.chars().count();
    if total <= MAX_OUTPUT_CHARS {
        return markdown;
    }
    let kept: String = markdown.chars().take(MAX_OUTPUT_CHARS).collect();
    let kept = match kept.rfind('\n') {
        Some(index) if index > MAX_OUTPUT_CHARS / 2 => &kept[..index],
        _ => kept.as_str(),
    };
    let omitted = total - kept.chars().count();
    format!(
        "{kept}\n… output truncated ({omitted} more chars); narrow the query or use --full-output"
    )
}

fn shorten(text: &str, hint: &str) -> String {
    let total = text.chars().count();
    if total <= MAX_TEXT_CHARS {
        return text.to_owned();
    }
    let head: String = text.chars().take(TEXT_HEAD_CHARS).collect();
    let tail: String = text.chars().skip(total - TEXT_TAIL_CHARS).collect();
    let omitted = total - TEXT_HEAD_CHARS - TEXT_TAIL_CHARS;
    format!("{head}\n… [{omitted} chars omitted; {hint}] …\n{tail}")
}

fn strip_ansi(text: &str) -> String {
    if !text.contains('\u{1b}') {
        return text.to_owned();
    }
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\u{1b}' {
            output.push(character);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_codes_trailing_blanks_and_blank_runs_are_removed() {
        let cleaned = clean_text("\u{1b}[1;31merror\u{1b}[0m: bad   \n\n\n\nnext\n");
        assert_eq!(cleaned, "error: bad\n\nnext");
    }

    #[test]
    fn long_text_keeps_head_and_tail() {
        let text = format!("{}MIDDLE{}", "a".repeat(5_000), "z".repeat(5_000));
        let cleaned = clean_text(&text);
        assert!(cleaned.starts_with("aaa"));
        assert!(cleaned.ends_with("zzz"));
        assert!(cleaned.contains("chars omitted"));
        assert!(cleaned.chars().count() < 4_000);
    }

    #[test]
    fn short_text_is_unchanged() {
        assert_eq!(clean_text("fine\ntext"), "fine\ntext");
    }

    #[test]
    fn oversized_output_is_cut_on_a_line_boundary() {
        let markdown = "line of output\n".repeat(3_000);
        let capped = cap_output(markdown);
        assert!(capped.chars().count() < 24_200);
        assert!(capped.contains("output truncated"));
        assert!(
            capped
                .lines()
                .filter(|line| *line == "line of output")
                .count()
                > 1_000
        );
        assert_eq!(cap_output("small".to_owned()), "small");
    }
}

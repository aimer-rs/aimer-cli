//! The block of generated entries between two marker comments.
//!
//! Everything Aimer writes lives between the markers and is rebuilt from
//! `Aimer.toml` on every sync. Everything outside them belongs to the user.

use anyhow::bail;

const START: &str = "<!-- aimer:permissions:start";
const END: &str = "<!-- aimer:permissions:end";

/// The comment that opens the generated block.
pub(super) fn start_comment() -> String {
    format!(
        "{START}: generated from [permissions] in Aimer.toml, edits between the markers are overwritten -->"
    )
}

/// The comment that closes the generated block.
pub(super) fn end_comment() -> String {
    format!("{END} -->")
}

/// `lines` between the two markers, one per line, each indented by `indent`
/// and ending in a newline.
pub(super) fn render(indent: &str, lines: &[String]) -> String {
    let mut block = String::new();
    for line in std::iter::once(start_comment())
        .chain(lines.iter().cloned())
        .chain(std::iter::once(end_comment()))
    {
        block.push_str(indent);
        block.push_str(&line);
        block.push('\n');
    }
    block
}

/// `text` without its generated block, and whether there was one.
///
/// Whole lines are removed, so a block added by [`render`] leaves no trace.
pub(super) fn strip(text: &str) -> anyhow::Result<(String, bool)> {
    let Some(start) = text.find(START) else {
        if text.contains(END) {
            bail!("found `{END}` without `{START}`; remove the stray marker or restore the block");
        }
        return Ok((text.to_string(), false));
    };
    let Some(end_at) = text[start..].find(END).map(|i| start + i) else {
        bail!("found `{START}` without `{END}`; restore the end marker or delete the block");
    };
    let end = text[end_at..]
        .find("-->")
        .map_or(text.len(), |i| end_at + i + 3);

    let line_start = match text[..start].rfind('\n') {
        Some(i) if text[i + 1..start].trim().is_empty() => i + 1,
        None if text[..start].trim().is_empty() => 0,
        _ => start,
    };
    let line_end = match text[end..].find('\n') {
        Some(i) if text[end..end + i].trim().is_empty() => end + i + 1,
        None if text[end..].trim().is_empty() => text.len(),
        _ => end,
    };
    Ok((
        format!("{}{}", &text[..line_start], &text[line_end..]),
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rendered_block_is_removed_without_a_trace() {
        let block = render("    ", &["<a/>".to_string()]);
        let text = format!("<x>\n{block}</x>\n");

        let (stripped, found) = strip(&text).unwrap();

        assert!(found);
        assert_eq!(stripped, "<x>\n</x>\n");
    }

    #[test]
    fn text_without_a_block_is_returned_as_is() {
        let (stripped, found) = strip("<x/>\n").unwrap();

        assert!(!found);
        assert_eq!(stripped, "<x/>\n");
    }

    #[test]
    fn a_lone_marker_is_an_error_instead_of_a_guess() {
        let lone_start = format!("<x>\n{}\n</x>", start_comment());
        let lone_end = format!("<x>\n{}\n</x>", end_comment());

        assert!(
            strip(&lone_start)
                .unwrap_err()
                .to_string()
                .contains("without")
        );
        assert!(
            strip(&lone_end)
                .unwrap_err()
                .to_string()
                .contains("without")
        );
    }
}
